//! Startup measured on a consistent copy of a store, in a unit that does not
//! depend on the filesystem or the machine, and attributed to the statements
//! the open itself executes.
//!
//! The copy: SQLite's online backup API from a read-only side connection,
//! page for page (free pages too), so it is the store as a clean shutdown
//! would leave it, and taking it neither writes nor checkpoints the live
//! store. The long-lived connection is not disturbed, so the main run can
//! measure startup daily without restarting core.
//!
//! The unit: SQLite page reads, one `pread` of one page per fetch that
//! misses the open's page cache (the default, as `Store::open` and
//! `ReceiptStore::open` leave it), counted as the thread's `syscr`
//! (`/proc/thread-self/io`). The open runs in a child process (this test
//! binary, re-run on `measure_in_child`), alone: the bundled SQLite is built
//! with `SQLITE_ENABLE_MEMORY_MANAGEMENT`, so every connection of a process
//! shares one page-cache LRU, and in the test process the live store's
//! connection and parallel tests would decide what the open finds cached.
//! Alone, for a given file, the count is the same on every run, on tmpfs or
//! disk, in debug or release; wall time and bytes read are information.
//!
//! The attribution: the store hands its connection to a test hook before any
//! statement of startup runs (`test_support::on_open`), and the child traces
//! every statement there (`trace_v2`: when it starts and when it finishes).
//! The page reads between two trace events belong to the statement running
//! (the innermost, when one runs inside another) or to no statement
//! (preparing, the schema); each statement is known by its own text, as
//! production wrote it, with its VM steps and full-scan steps. Nothing of
//! production's SQL is copied here. Each read-only statement is then run
//! again on its own fresh connection to a second copy, cold, for the series
//! the trend checks fit (a statement read cold reads at least as many pages
//! as inside the open, where the cache is shared and one statement's scan
//! can evict another's pages), and its query plan is read there.
use super::{
    executor::Opened,
    sim::{Sim, Startup, sizes},
    support::Io,
};
use limeos_executor_container::ReceiptStore;
use limeos_persistence::{Store, test_support};
use rusqlite::{
    MAIN_DB, OpenFlags, StatementStatus,
    trace::{TraceEvent, TraceEventCodes},
};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::HashMap,
    os::unix::fs::{DirBuilderExt, FileExt, PermissionsExt},
    path::{Path, PathBuf},
    time::Instant,
};

/// `source`'s committed content (database and WAL) as a single file at
/// `target`.
pub fn copy(source: &Path, target: &Path) {
    let from = super::checks::reader(source).expect("side connection");
    from.backup(MAIN_DB, target, None::<fn(rusqlite::backup::Progress)>)
        .expect("online backup of the store");
    drop(from);
    // Both stores refuse a file other users could read.
    std::fs::set_permissions(target, std::fs::Permissions::from_mode(0o600)).unwrap();
}

/// A private directory for a copy, removed with everything in it afterwards.
fn scratch(sim: &Sim, name: &str) -> PathBuf {
    let dir = sim._dir.path().join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
    dir
}

/// `Store::open` on a copy of the live core store, measured and traced alone
/// in a child process; the copies are then deleted. Startup checks run as at
/// a restart, recovery included.
pub fn core(sim: &mut Sim) {
    let dir = scratch(sim, "startup-core");
    let path = dir.join("core.sqlite");
    copy(&sim.core_path, &path);
    let (file, wal) = sizes(&path);
    let child = in_child("core", &path);
    if let Some(refused) = &child.refused {
        sim.violation(
            super::checks::STARTUP,
            format!("a copy of the core store refused to open: {refused}"),
        );
    }
    std::fs::remove_dir_all(&dir).unwrap();
    sim.core_starts.push(Startup {
        at: sim.now(),
        reason: COPY,
        file,
        wal,
        opened: child.opened,
        trace: child.trace,
        replay: 0,
    });
}

/// WAL frames a reopen replays: the frames after the WAL's header that carry
/// its current salts, up to the last one that commits a transaction (a
/// nonzero database size). Frames of an earlier generation (the WAL is reused
/// from its start after a checkpoint) carry other salts and are ignored, as
/// SQLite's recovery ignores them.
pub fn committed_frames(wal: &[u8]) -> u64 {
    const HEADER: usize = 32;
    const FRAME_HEADER: usize = 24;
    let word = |bytes: &[u8], at: usize| -> Option<u32> {
        Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
    };
    let (Some(page), Some(salt1), Some(salt2)) = (word(wal, 8), word(wal, 16), word(wal, 20))
    else {
        return 0;
    };
    let frame = FRAME_HEADER + page as usize;
    let (mut at, mut seen, mut committed) = (HEADER, 0, 0);
    while at + frame <= wal.len() {
        if word(wal, at + 8) != Some(salt1) || word(wal, at + 12) != Some(salt2) {
            break;
        }
        seen += 1;
        if word(wal, at + 4).is_some_and(|size| size > 0) {
            committed = seen;
        }
        at += frame;
    }
    committed
}

/// The reason of an open measured on a crash image.
pub const CRASH: &str = "measured on a crash image";

/// `Store::open` on core's store as a crash leaves it now (the database and
/// its WAL as they are on disk, no wal-index), measured and traced alone in a
/// child process like the nightly copies, with each statement run cold on a
/// consistent copy taken at the same moment. Besides the startup checks,
/// the open must keep every committed transaction: the jobs the live store
/// holds, and at least its events.
pub fn core_crash(sim: &mut Sim) {
    let dir = scratch(sim, "startup-crash");
    let path = dir.join("core.sqlite");
    let image = sim.crash_image();
    std::fs::write(&path, &image.db).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let frames = image.wal.as_deref().map_or(0, committed_frames);
    if let Some(wal) = &image.wal {
        let wal_path = super::sim::suffixed(&path, "-wal");
        std::fs::write(&wal_path, wal).unwrap();
        std::fs::set_permissions(&wal_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let cold = dir.join("core.consistent.sqlite");
    copy(&sim.core_path, &cold);
    let committed = Durable::read(&super::checks::reader(&sim.core_path).unwrap(), None);
    let (file, wal) = sizes(&path);
    let child = in_child_with("core", &path, Some((&cold, committed.last)));
    if let Some(refused) = &child.refused {
        sim.violation(
            super::checks::STARTUP,
            format!("a crash image of the core store refused to open: {refused}"),
        );
    }
    let lost = match &child.held {
        Some(reopened) => lost_in_crash(&committed, reopened),
        None => vec!["the child read nothing back".to_owned()],
    };
    if child.refused.is_none() && !lost.is_empty() {
        sim.violation(
            super::checks::STARTUP,
            format!(
                "a crash image of the core store lost committed work in its open ({frames} WAL frames to replay): {}",
                lost.join("; ")
            ),
        );
    }
    std::fs::remove_dir_all(&dir).unwrap();
    sim.core_starts.push(Startup {
        at: sim.now(),
        reason: CRASH,
        file,
        wal,
        opened: child.opened,
        trace: child.trace,
        replay: frames,
    });
}

/// What a store holds that a crash must not lose, read on a side
/// connection: every job (a digest of the ids, with their count) and every
/// event through cursor `last`, by identity and content (a digest of each
/// row's cursor, principal, job, kind and time, in cursor order, with their
/// count), the jobs startup recovery must mark (`running` or `verifying`),
/// what the audit counter says, and every event after `last` (job and kind).
///
/// `Store::open` adds events and removes none, but in one case: when the
/// recovery events would not fit within the audit reserve past the cap it
/// prunes first (`make_room`). Job-row retention, like every other rule but
/// that one, runs only on the writer thread, after the open; the child opens
/// with `Store::open` alone. So an open that does not need `make_room` must
/// keep every committed job and event exactly as it was, and may add only one
/// `job_recovery_required` event for each interrupted job, after `last`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Durable {
    pub jobs: String,
    pub last: i64,
    pub events: u64,
    pub digest: String,
    pub interrupted: Vec<String>,
    pub audit: i64,
    pub added: Vec<(Option<String>, String)>,
}
impl Durable {
    /// The store as `conn` sees it; events after `through` (every event when
    /// `None`) are listed in `added` rather than digested.
    pub fn read(conn: &rusqlite::Connection, through: Option<i64>) -> Self {
        let strings = |sql: &str| -> Vec<String> {
            conn.prepare(sql)
                .and_then(|mut s| s.query_map([], |r| r.get(0))?.collect())
                .unwrap_or_default()
        };
        let ids = strings("SELECT id FROM jobs ORDER BY id");
        let interrupted =
            strings("SELECT id FROM jobs WHERE state IN ('running','verifying') ORDER BY id");
        let last = through.unwrap_or_else(|| {
            conn.query_row("SELECT coalesce(max(cursor),0) FROM events", [], |r| {
                r.get(0)
            })
            .unwrap_or(0)
        });
        let mut rows = String::new();
        let mut events = 0;
        let mut statement = conn
            .prepare("SELECT cursor,principal,job,kind,created FROM events WHERE cursor<=? ORDER BY cursor")
            .unwrap();
        let mut query = statement.query([last]).unwrap();
        while let Some(row) = query.next().unwrap() {
            events += 1;
            rows.push_str(&format!(
                "{}|{:?}|{:?}|{}|{}\n",
                row.get::<_, i64>(0).unwrap(),
                row.get::<_, Option<String>>(1).unwrap(),
                row.get::<_, Option<String>>(2).unwrap(),
                row.get::<_, String>(3).unwrap(),
                row.get::<_, i64>(4).unwrap()
            ));
        }
        drop(query);
        let added = conn
            .prepare("SELECT job,kind FROM events WHERE cursor>? ORDER BY cursor")
            .and_then(|mut s| {
                s.query_map([last], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect()
            })
            .unwrap_or_default();
        Self {
            jobs: format!(
                "{} ({})",
                limeos_identity::digest(&ids.join(",")),
                ids.len()
            ),
            last,
            events,
            digest: limeos_identity::digest(&rows),
            interrupted,
            audit: conn
                .query_row("SELECT audit_bytes FROM meta WHERE id=1", [], |r| r.get(0))
                .unwrap_or(0),
            added,
        }
    }
    fn to_json(&self) -> Value {
        json!({
            "jobs": self.jobs,
            "last": self.last,
            "events": self.events,
            "digest": self.digest,
            "interrupted": self.interrupted,
            "audit": self.audit,
            "added": self.added.iter().map(|(job, kind)| json!([job, kind])).collect::<Vec<_>>(),
        })
    }
    fn from_json(value: &Value) -> Option<Self> {
        let strings = |v: &Value| -> Vec<String> {
            v.as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default()
        };
        Some(Self {
            jobs: value["jobs"].as_str()?.to_owned(),
            last: value["last"].as_i64()?,
            events: value["events"].as_u64()?,
            digest: value["digest"].as_str()?.to_owned(),
            interrupted: strings(&value["interrupted"]),
            audit: value["audit"].as_i64()?,
            added: value["added"]
                .as_array()?
                .iter()
                .map(|pair| {
                    Some((
                        pair[0].as_str().map(str::to_owned),
                        pair[1].as_str()?.to_owned(),
                    ))
                })
                .collect::<Option<_>>()?,
        })
    }
}

/// What an open of a crash image lost or changed of what was `committed`
/// (`Durable`), or added beyond one recovery event per interrupted job:
/// empty when it kept everything. Opens that needed `make_room` (recovery
/// beyond the audit reserve) are refused here outright: the soak keeps the
/// events table at its budget, half the cap, so it never takes one.
pub fn lost_in_crash(committed: &Durable, reopened: &Durable) -> Vec<String> {
    let mut lost = Vec::new();
    let reserve = limeos_persistence::AuditUsage::LIMIT + test_support::AUDIT_RESERVE;
    let recovery = committed.interrupted.len() as i64 * test_support::EVENT_BYTES;
    if !committed.interrupted.is_empty() && committed.audit + recovery > reserve {
        lost.push(format!(
            "the image needed make_room ({} audit bytes and {} interrupted jobs): every event cannot be held to",
            committed.audit,
            committed.interrupted.len()
        ));
    }
    if reopened.jobs != committed.jobs {
        lost.push(format!(
            "jobs {} (committed {})",
            reopened.jobs, committed.jobs
        ));
    }
    if reopened.last != committed.last
        || reopened.events != committed.events
        || reopened.digest != committed.digest
    {
        lost.push(format!(
            "{} events through cursor {} digest to {} (committed: {} events, {})",
            reopened.events, reopened.last, reopened.digest, committed.events, committed.digest
        ));
    }
    let recovered = serde_json::to_string(&limeos_domain::EventKind::JobRecoveryRequired).unwrap();
    let mut marked: Vec<String> = Vec::new();
    for (job, kind) in &reopened.added {
        match job {
            Some(job) if *kind == recovered && committed.interrupted.contains(job) => {
                marked.push(job.clone());
            }
            _ => lost.push(format!("an event the open may not add: {kind} for {job:?}")),
        }
    }
    marked.sort();
    let unique = marked.len();
    marked.dedup();
    if marked.len() != unique || marked != committed.interrupted {
        lost.push(format!(
            "{unique} recovery events for {} interrupted jobs",
            committed.interrupted.len()
        ));
    }
    lost
}

/// `ReceiptStore::open` on a copy of the live receipt store, measured
/// alone in a child process.
pub fn receipts(sim: &mut Sim) {
    let dir = scratch(sim, "startup-receipts");
    let path = dir.join("receipts.sqlite");
    copy(&sim.exec.dir.join("receipts.sqlite"), &path);
    let (file, wal) = sizes(&path);
    let now = sim.now();
    let child = in_child("receipts", &path);
    if let Some(refused) = &child.refused {
        sim.violation(
            super::checks::STARTUP,
            format!("a copy of the receipt store refused to open: {refused}"),
        );
    }
    std::fs::remove_dir_all(&dir).unwrap();
    sim.executor_starts.push(Startup {
        at: now,
        reason: COPY,
        file,
        wal,
        opened: child.opened,
        trace: None,
        replay: 0,
    });
}

/// One b-tree a statement's program walks from one end rather than seeking
/// into: a cursor opened on it (`OpenRead`/`OpenWrite`) and then positioned by
/// `Rewind` or `Last`, or counted whole by `Count`. Read from the statement's
/// own bytecode (`EXPLAIN`), so every node of its plan is judged, under any
/// alias and whatever an `INDEXED BY`/`NOT INDEXED` clause says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scan {
    /// The b-tree: a table, or an index (`index` true) of `table`.
    pub btree: String,
    pub table: String,
    pub index: bool,
    /// The opcode that walks it: `Rewind`, `Last` or `Count`.
    pub op: String,
}

/// One statement core's open executed, as the trace saw it.
#[derive(Clone, Debug, Default)]
pub struct Statement {
    /// Its text, as production wrote it.
    pub sql: String,
    pub runs: u64,
    /// Page reads while it ran, inside the open.
    pub reads: u64,
    /// VM steps and full-scan steps (`SQLITE_STMTSTATUS_VM_STEP`,
    /// `SQLITE_STMTSTATUS_FULLSCAN_STEP`) over all its runs in the open:
    /// deterministic for a given file, unlike page reads.
    pub vm_steps: u64,
    pub full_scans: u64,
    /// Page reads run alone and cold, if it only reads.
    pub cold: Option<u64>,
    /// Its query plan (`EXPLAIN QUERY PLAN` details), for reports.
    pub plan: Vec<String>,
    /// Every b-tree its program walks whole (`Scan`).
    pub scans: Vec<Scan>,
    /// Why its bytecode could not be read, if it could not: its walks are
    /// then unknown, which the checks refuse rather than read as none.
    pub unread: Option<String>,
}
impl Statement {
    /// The storage-mount proof check: the startup statement that reads
    /// `storage_mount_results`, the table no other startup statement reads
    /// (known by production's own text).
    pub fn proof(&self) -> bool {
        self.sql.contains("storage_mount_results")
    }
    /// Whether its plan looks events up by kind (`events_kind`).
    pub fn events_by_kind(&self) -> bool {
        self.plan
            .iter()
            .any(|step| step.starts_with("SEARCH") && step.contains("USING INDEX events_kind"))
    }
    /// The tables whose b-trees (the table's, or an index's) its program
    /// walks whole, each once.
    pub fn walked(&self) -> Vec<&str> {
        let mut tables: Vec<&str> = self.scans.iter().map(|scan| scan.table.as_str()).collect();
        tables.sort_unstable();
        tables.dedup();
        tables
    }
    /// Its text with every run of whitespace made one space.
    pub fn text(&self) -> String {
        self.sql.split_whitespace().collect::<Vec<_>>().join(" ")
    }
    /// Its text, shortened, for a report.
    pub fn short(&self) -> String {
        self.text().chars().take(100).collect()
    }
}

/// What core's open on one copy executed.
#[derive(Clone, Debug, Default)]
pub struct Trace {
    pub statements: Vec<Statement>,
    /// Page reads outside every statement: preparing, the schema.
    pub outside: u64,
    /// Rows of every table of the copy, as the open found it.
    pub rows: std::collections::BTreeMap<String, u64>,
}
impl Trace {
    /// The pinned defect: a storage-mount proof check that looks events up
    /// by kind.
    pub fn defect(&self) -> Option<&Statement> {
        self.statements
            .iter()
            .filter(|s| s.proof() && s.events_by_kind())
            .max_by_key(|s| s.cold.unwrap_or(s.reads))
    }
    /// The cold page reads of every read-only statement.
    pub fn cold(&self) -> u64 {
        self.statements.iter().filter_map(|s| s.cold).sum()
    }
    /// The same, but the pinned defect's.
    pub fn cold_but_defect(&self) -> u64 {
        self.cold() - self.defect().and_then(|s| s.cold).unwrap_or(0)
    }
    /// Page reads inside the open (statements and outside).
    pub fn reads(&self) -> u64 {
        self.statements.iter().map(|s| s.reads).sum::<u64>() + self.outside
    }
    /// VM steps of every statement but the pinned defect's.
    pub fn vm_steps_but_defect(&self) -> u64 {
        let defect = self.defect().map(|s| s.sql.as_str());
        self.statements
            .iter()
            .filter(|s| Some(s.sql.as_str()) != defect)
            .map(|s| s.vm_steps)
            .sum()
    }
    pub fn rows_of(&self, table: &str) -> u64 {
        self.rows.get(table).copied().unwrap_or(0)
    }
}

/// What a child process measured.
struct Child {
    opened: Opened,
    trace: Option<Trace>,
    refused: Option<String>,
    /// What the opened store held afterwards (`Durable`), for core.
    held: Option<Durable>,
}

const MEASURE: &str = "LIMEOS_SOAK_MEASURE";

/// Runs `measure_in_child` in a new process of this binary on `path`.
fn in_child(store: &str, path: &Path) -> Child {
    in_child_with(store, path, None)
}
/// `in_child`, with each statement run cold on `cold` (a consistent copy of
/// the same content) rather than on a copy of `path` the child makes, and
/// the events after the given cursor (the last committed) read back apart.
fn in_child_with(store: &str, path: &Path, cold: Option<(&Path, i64)>) -> Child {
    let mut request = format!("{store}\n{}", path.display());
    if let Some((cold, last)) = cold {
        request.push_str(&format!("\n{last}\n{}", cold.display()));
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "soak::startup::measure_in_child",
            "--exact",
            "--ignored",
            "--nocapture",
            "--test-threads",
            "1",
        ])
        .env(MEASURE, request)
        .output()
        .expect("a child process of the soak binary");
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text
        .lines()
        .find_map(|line| line.split_once("MEASURED ").map(|(_, result)| result))
        .unwrap_or_else(|| {
            panic!(
                "the measuring child printed no result ({}):\n{text}\n{}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            )
        });
    let value: Value = serde_json::from_str(line).expect("the child's measurement");
    let number = |v: &Value| v.as_u64().unwrap_or(0);
    let trace = value["statements"].as_array().map(|statements| Trace {
        statements: statements
            .iter()
            .map(|s| Statement {
                sql: s["sql"].as_str().unwrap_or_default().to_owned(),
                runs: number(&s["runs"]),
                reads: number(&s["reads"]),
                vm_steps: number(&s["vm"]),
                full_scans: number(&s["fullscan"]),
                cold: s["cold"].as_u64(),
                plan: s["plan"]
                    .as_array()
                    .map(|p| {
                        p.iter()
                            .filter_map(|x| x.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default(),
                scans: s["scans"]
                    .as_array()
                    .map(|scans| {
                        scans
                            .iter()
                            .map(|x| Scan {
                                btree: x["btree"].as_str().unwrap_or_default().to_owned(),
                                table: x["table"].as_str().unwrap_or_default().to_owned(),
                                index: x["index"].as_bool().unwrap_or(false),
                                op: x["op"].as_str().unwrap_or_default().to_owned(),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                unread: s["unread"].as_str().map(str::to_owned),
            })
            .collect(),
        outside: number(&value["outside"]),
        rows: value["rows"]
            .as_object()
            .map(|rows| {
                rows.iter()
                    .map(|(table, n)| (table.clone(), number(n)))
                    .collect()
            })
            .unwrap_or_default(),
    });
    Child {
        opened: Opened {
            io: Io {
                rchar: number(&value["rchar"]),
                syscr: number(&value["syscr"]),
                ..Io::default()
            },
            elapsed: std::time::Duration::from_micros(number(&value["micros"])),
        },
        trace,
        refused: value["refused"].as_str().map(str::to_owned),
        held: Durable::from_json(&value["durable"]),
    }
}

/// The thread's `syscr`, read with one `pread` of `/proc/thread-self/io`
/// (which the next reading counts: one read call each).
struct Reads(std::fs::File);
impl Reads {
    fn open() -> Self {
        Self(std::fs::File::open("/proc/thread-self/io").expect("per-thread I/O accounting"))
    }
    fn now(&self) -> u64 {
        let mut buffer = [0u8; 4096];
        let len = self.0.read_at(&mut buffer, 0).expect("per-thread I/O");
        std::str::from_utf8(&buffer[..len])
            .ok()
            .and_then(|text| {
                text.lines()
                    .find_map(|line| line.strip_prefix("syscr: "))
                    .and_then(|value| value.trim().parse().ok())
            })
            .expect("syscr in /proc/thread-self/io")
    }
}

/// The trace of one open: page reads between trace events, attributed.
struct Tracer {
    reads: Reads,
    last: u64,
    /// Statements running, innermost last, with their VM and full-scan step
    /// counters when they started (a cached statement's counters run on
    /// across its runs).
    running: Vec<(String, u64, u64)>,
    order: Vec<String>,
    statements: HashMap<String, (Statement, Option<String>)>,
    outside: u64,
}
impl Tracer {
    /// Page reads since the last reading, less that reading's own call.
    fn delta(&mut self) -> u64 {
        let now = self.reads.now();
        let delta = (now - self.last).saturating_sub(1);
        self.last = now;
        delta
    }
    fn attribute(&mut self, reads: u64) {
        match self.running.last() {
            Some((sql, _, _)) => self.statements.get_mut(sql).unwrap().0.reads += reads,
            None => self.outside += reads,
        }
    }
}
thread_local! {
    static TRACER: RefCell<Option<Tracer>> = const { RefCell::new(None) };
}
fn traced(event: TraceEvent<'_>) {
    TRACER.with(|tracer| {
        let mut tracer = tracer.borrow_mut();
        let Some(tracer) = tracer.as_mut() else {
            return;
        };
        let reads = tracer.delta();
        tracer.attribute(reads);
        let status =
            |stmt: &rusqlite::trace::StmtRef<'_>, s| u64::try_from(stmt.get_status(s)).unwrap_or(0);
        match event {
            TraceEvent::Stmt(stmt, _) => {
                let sql = stmt.sql().into_owned();
                // A trigger's program starts inside its statement's run.
                if tracer.running.last().map(|r| &r.0) != Some(&sql) {
                    tracer.running.push((
                        sql.clone(),
                        status(&stmt, StatementStatus::VmStep),
                        status(&stmt, StatementStatus::FullscanStep),
                    ));
                }
                if !tracer.statements.contains_key(&sql) {
                    tracer.order.push(sql.clone());
                    let statement = Statement {
                        sql: sql.clone(),
                        ..Statement::default()
                    };
                    tracer
                        .statements
                        .insert(sql, (statement, stmt.expanded_sql()));
                }
            }
            TraceEvent::Profile(stmt, _) => {
                let sql = stmt.sql().into_owned();
                let started = tracer
                    .running
                    .iter()
                    .rposition(|s| s.0 == sql)
                    .map(|at| tracer.running.remove(at));
                if let Some((statement, _)) = tracer.statements.get_mut(&sql) {
                    statement.runs += 1;
                    let (vm, scans) = started.map_or((0, 0), |(_, vm, scans)| (vm, scans));
                    statement.vm_steps += status(&stmt, StatementStatus::VmStep).saturating_sub(vm);
                    statement.full_scans +=
                        status(&stmt, StatementStatus::FullscanStep).saturating_sub(scans);
                }
            }
            _ => {}
        }
    });
}
fn hook(conn: &mut rusqlite::Connection) {
    conn.trace_v2(
        TraceEventCodes::SQLITE_TRACE_STMT | TraceEventCodes::SQLITE_TRACE_PROFILE,
        Some(traced),
    )
    .expect("statement trace on the store's connection");
}

/// Every b-tree `sql`'s program walks whole (`Scan`), read from its bytecode
/// on `conn`: each cursor opened on the main database (`OpenRead`,
/// `OpenWrite`, `ReopenIdx` with a literal root page) is named by its root
/// page through `sqlite_schema`, and each `Rewind`, `Last` or `Count` on such
/// a cursor is a walk of that b-tree from one end. A cursor number reopened
/// on anything else (an ephemeral table, a sorter, a register-rooted
/// b-tree) forgets its b-tree. An error if the program cannot be read.
pub(super) fn scans(conn: &rusqlite::Connection, sql: &str) -> Result<Vec<Scan>, String> {
    /// `OPFLAG_P2ISREG` (sqlite3.c): the root page is in a register, not
    /// literal.
    const P2_IS_REGISTER: i64 = 0x10;
    let mut roots: HashMap<i64, (String, String, bool)> = conn
        .prepare("SELECT rootpage,name,tbl_name,type='index' FROM sqlite_schema WHERE rootpage>0")
        .and_then(|mut s| {
            s.query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?, r.get(3)?))))?
                .collect()
        })
        .unwrap_or_default();
    roots.insert(1, ("sqlite_schema".into(), "sqlite_schema".into(), false));
    let mut program = conn
        .prepare(&format!("EXPLAIN {sql}"))
        .map_err(|e| e.to_string())?;
    let rows = program
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(6)?,
            ))
        })
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
        .map_err(|e| e.to_string())?;
    let mut cursors: HashMap<i64, i64> = HashMap::new();
    let mut found = Vec::new();
    for (op, p1, p2, p3, p5) in rows {
        match op.as_str() {
            "OpenRead" | "OpenWrite" | "ReopenIdx" if p3 == 0 && p5 & P2_IS_REGISTER == 0 => {
                cursors.insert(p1, p2);
            }
            _ if op.starts_with("Open") || op == "SorterOpen" => {
                cursors.remove(&p1);
            }
            "Rewind" | "Last" | "Count" => {
                if let Some((btree, table, index)) = cursors.get(&p1).and_then(|r| roots.get(r)) {
                    let scan = Scan {
                        btree: btree.clone(),
                        table: table.clone(),
                        index: *index,
                        op,
                    };
                    if !found.contains(&scan) {
                        found.push(scan);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(found)
}

/// Each statement run alone and cold on its own connection to `path` (if it
/// only reads), its plan and the b-trees its program walks.
fn cold(path: &Path, statements: &mut [(Statement, Option<String>)]) {
    let reads = Reads::open();
    for (statement, expanded) in statements {
        // A statement prepared and run inside another's run (the
        // `PRAGMA foreign_key_check` behind `pragma_foreign_key_check`) has
        // its expanded text commented out line by line ("-- "): its program
        // is read from its own text, and it is not run cold, its reads being
        // inside its parent's cold run already.
        let nested = expanded.as_deref().is_some_and(|e| e.starts_with("-- "));
        let sql = match expanded {
            Some(expanded) if !nested => expanded.clone(),
            _ => statement.sql.clone(),
        };
        // Its bytecode, on a connection of its own (closed before the cold
        // run, so that run starts with as empty a cache as before).
        let program = rusqlite::Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .expect("a side connection to the copy");
        match scans(&program, &sql) {
            Ok(found) => statement.scans = found,
            Err(why) => statement.unread = Some(why),
        }
        drop(program);
        let conn = rusqlite::Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .expect("a side connection to the copy");
        statement.plan = conn
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .and_then(|mut s| s.query_map([], |r| r.get::<_, String>(3))?.collect())
            .unwrap_or_default();
        if nested {
            continue;
        }
        let Ok(mut prepared) = conn.prepare(&sql) else {
            continue;
        };
        if !prepared.readonly() {
            continue;
        }
        let before = reads.now();
        let mut rows = prepared.raw_query();
        let complete = loop {
            match rows.next() {
                Ok(Some(_)) => {}
                Ok(None) => break true,
                Err(_) => break false,
            }
        };
        let after = reads.now();
        if complete {
            statement.cold = Some((after - before).saturating_sub(1));
        }
    }
}

/// Rows of every table of the store at `path`.
fn table_rows(path: &Path) -> std::collections::BTreeMap<String, u64> {
    let conn = super::checks::reader(path).expect("a side connection to the copy");
    let mut names: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_schema WHERE type='table'")
        .and_then(|mut s| s.query_map([], |r| r.get(0))?.collect())
        .unwrap_or_default();
    names.push("sqlite_schema".into());
    names
        .into_iter()
        .filter_map(|name| {
            let rows: i64 = conn
                .query_row(&format!("SELECT count(*) FROM \"{name}\""), [], |r| {
                    r.get(0)
                })
                .ok()?;
            Some((name, u64::try_from(rows).unwrap_or(0)))
        })
        .collect()
}

/// The child half of `in_child`: a no-op unless run by it.
#[test]
#[ignore = "the child half of an isolated startup measurement, run by the soak itself"]
fn measure_in_child() {
    let Some(request) = std::env::var_os(MEASURE) else {
        return;
    };
    let request = request.into_string().unwrap();
    let mut parts = request.splitn(4, '\n');
    let (store, path, last, consistent) = (
        parts.next().unwrap(),
        PathBuf::from(parts.next().unwrap()),
        parts.next().map(|last| last.parse::<i64>().unwrap()),
        parts.next().map(PathBuf::from),
    );
    let result = if store == "core" {
        // A second copy, for each statement run cold: the open writes the
        // first (recovery, the generation). A crash image's comes with it.
        let second = consistent.unwrap_or_else(|| {
            let second = path.with_extension("cold.sqlite");
            std::fs::copy(&path, &second).unwrap();
            second
        });
        let (io, at) = (Io::now(), Instant::now());
        let reads = Reads::open();
        let started = reads.now();
        TRACER.with(|t| {
            *t.borrow_mut() = Some(Tracer {
                last: started,
                reads,
                running: Vec::new(),
                order: Vec::new(),
                statements: HashMap::new(),
                outside: 0,
            });
        });
        test_support::on_open(Some(hook));
        let opened = Store::open(&path).map(drop);
        test_support::on_open(None);
        let mut tracer = TRACER.with(|t| t.borrow_mut().take()).unwrap();
        let reads = tracer.delta();
        tracer.attribute(reads);
        let elapsed = at.elapsed();
        let rchar = Io::now().since(io).rchar;
        let mut statements: Vec<(Statement, Option<String>)> = tracer
            .order
            .iter()
            .map(|sql| tracer.statements.remove(sql).unwrap())
            .collect();
        cold(&second, &mut statements);
        let rows = table_rows(&second);
        let held = Durable::read(&super::checks::reader(&path).unwrap(), last);
        let total = statements.iter().map(|(s, _)| s.reads).sum::<u64>() + tracer.outside;
        json!({
            "syscr": total,
            "rchar": rchar,
            "micros": elapsed.as_micros() as u64,
            "refused": opened.err().map(|e| format!("{e:?}")),
            "outside": tracer.outside,
            "rows": rows,
            "durable": held.to_json(),
            "statements": statements.iter().map(|(s, _)| json!({
                "sql": s.sql,
                "runs": s.runs,
                "reads": s.reads,
                "vm": s.vm_steps,
                "fullscan": s.full_scans,
                "cold": s.cold,
                "plan": s.plan,
                "scans": s.scans.iter().map(|scan| json!({
                    "btree": scan.btree,
                    "table": scan.table,
                    "index": scan.index,
                    "op": scan.op,
                })).collect::<Vec<_>>(),
                "unread": s.unread,
            })).collect::<Vec<_>>(),
        })
    } else {
        let dir = path.parent().unwrap().to_path_buf();
        let (io, at) = (Io::now(), Instant::now());
        let opened = ReceiptStore::open(&dir).map(drop);
        let (io, elapsed) = (Io::now().since(io), at.elapsed());
        json!({
            "syscr": io.syscr,
            "rchar": io.rchar,
            "micros": elapsed.as_micros() as u64,
            "refused": opened.err().map(|e| format!("{e:?}")),
        })
    };
    println!("MEASURED {result}");
}

/// The reason a measured open was on a copy (the deterministic series).
pub const COPY: &str = "measured on a copy";

/// SQLite page reads of an open measured on a copy.
pub fn pages(start: &Startup) -> u64 {
    start.opened.io.syscr
}

/// The bytecode reader names a walk whatever the statement calls its table,
/// tells an index walk from a table walk, sees no walk in a seek, and says
/// when it cannot read a program instead of finding nothing.
#[test]
fn walks_are_read_from_bytecode_under_any_alias() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE events(id INTEGER PRIMARY KEY, kind TEXT, created INTEGER);
         CREATE INDEX events_kind ON events(kind);
         CREATE TABLE meta(id INTEGER PRIMARY KEY, audit_bytes INTEGER);",
    )
    .unwrap();
    let walks = |sql: &str| -> Vec<(String, String, bool)> {
        scans(&conn, sql)
            .unwrap()
            .into_iter()
            .map(|s| (s.btree, s.table, s.index))
            .collect()
    };
    let events = ("events".to_owned(), "events".to_owned(), false);
    assert_eq!(
        walks("SELECT sum(e.created) FROM events e NOT INDEXED"),
        std::slice::from_ref(&events)
    );
    assert_eq!(
        walks(
            "SELECT audit_bytes,(SELECT 0*count(x.created) FROM events AS x NOT INDEXED WHERE x.kind>'') FROM meta WHERE id=1"
        ),
        [events]
    );
    let counted = walks("SELECT count(*) FROM events");
    assert!(
        counted
            .iter()
            .all(|(_, table, index)| table == "events" && *index),
        "count(*) walks the smallest index, not the table: {counted:?}"
    );
    assert!(!counted.is_empty());
    assert_eq!(walks("SELECT created FROM events WHERE id=7"), []);
    assert_eq!(walks("SELECT id FROM events WHERE kind='login'"), []);
    assert!(scans(&conn, "SELECT nothing FROM no_such_table").is_err());
}

/// The crash check holds every committed event to its identity and content:
/// a reopened store missing one, or with one changed, fails it, whatever
/// the open added; one recovery event per interrupted job passes, and any
/// other addition fails. Run on the three tables the check reads, as the
/// open would leave them.
#[test]
fn a_crash_image_must_keep_every_committed_event_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        r#"CREATE TABLE jobs(id TEXT PRIMARY KEY, state TEXT);
         CREATE TABLE events(cursor INTEGER PRIMARY KEY AUTOINCREMENT, principal TEXT, job TEXT, kind TEXT, created INTEGER);
         CREATE TABLE meta(id INTEGER PRIMARY KEY, audit_bytes INTEGER);
         INSERT INTO meta VALUES(1,1024);
         INSERT INTO jobs VALUES('a','running'),('b','succeeded');
         INSERT INTO events(principal,job,kind,created) VALUES
             ('u','a','{"kind":"job_queued"}',10),('u','b','{"kind":"job_queued"}',11),
             ('u',NULL,'{"kind":"session_created"}',12),('u','b','{"kind":"job_transition","state":"succeeded"}',13);"#,
    )
    .unwrap();
    let committed = Durable::read(&conn, None);
    assert_eq!((committed.events, committed.last), (4, 4));
    assert_eq!(committed.interrupted, ["a"]);
    let reopened = |mutation: &str| -> Vec<String> {
        let image = dir.path().join("image.sqlite");
        std::fs::copy(&path, &image).unwrap();
        let copy = rusqlite::Connection::open(&image).unwrap();
        copy.execute_batch(mutation).unwrap();
        lost_in_crash(&committed, &Durable::read(&copy, Some(committed.last)))
    };
    let recovery = r#"INSERT INTO events(principal,job,kind,created) VALUES('u','a','{"kind":"job_recovery_required"}',20);"#;
    // Kept, with the open's one recovery event.
    assert_eq!(reopened(recovery), Vec::<String>::new());
    // Without it, or with two, the open did not do its recovery.
    assert_eq!(reopened("").len(), 1);
    assert_eq!(reopened(&format!("{recovery}{recovery}")).len(), 1);
    // One committed event dropped: its count and digest say so, although the
    // recovery event leaves the table as large as it was.
    let dropped = reopened(&format!("DELETE FROM events WHERE cursor=3;{recovery}"));
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    assert!(
        dropped[0].contains("3 events through cursor 4"),
        "{dropped:?}"
    );
    // One changed in place: same count, another digest.
    let changed = reopened(&format!(
        "UPDATE events SET created=14 WHERE cursor=4;{recovery}"
    ));
    assert_eq!(changed.len(), 1, "{changed:?}");
    // Any other addition, and a lost job.
    assert_eq!(
        reopened(&format!(
            r#"{recovery}INSERT INTO events(principal,job,kind,created) VALUES('u',NULL,'{{"kind":"session_created"}}',21);"#
        ))
        .len(),
        1
    );
    assert_eq!(
        reopened(&format!("{recovery}INSERT INTO jobs VALUES('c','queued');")).len(),
        1
    );
    // An image whose recovery needed make_room is not one the check can hold.
    let full = Durable {
        audit: limeos_persistence::AuditUsage::LIMIT + test_support::AUDIT_RESERVE,
        ..committed.clone()
    };
    let image = dir.path().join("image.sqlite");
    std::fs::copy(&path, &image).unwrap();
    let copy = rusqlite::Connection::open(&image).unwrap();
    copy.execute_batch(recovery).unwrap();
    assert_eq!(
        lost_in_crash(&full, &Durable::read(&copy, Some(committed.last))).len(),
        1
    );
}
