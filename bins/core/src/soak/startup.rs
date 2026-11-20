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
//! `ReceiptStore::open_at` leave it), counted as the thread's `syscr`
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
    from.backup(MAIN_DB, target, None)
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
    let child = in_child("core", &path, sim.now());
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
    });
}

/// `ReceiptStore::open_at` on a copy of the live receipt store, measured
/// alone in a child process.
pub fn receipts(sim: &mut Sim) {
    let dir = scratch(sim, "startup-receipts");
    let path = dir.join("receipts.sqlite");
    copy(&sim.exec.dir.join("receipts.sqlite"), &path);
    let (file, wal) = sizes(&path);
    let now = sim.now();
    let child = in_child("receipts", &path, now);
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
    });
}

/// One statement core's open executed, as the trace saw it.
#[derive(Clone, Debug, Default)]
pub struct Statement {
    /// Its text, as production wrote it.
    pub sql: String,
    pub runs: u64,
    /// Page reads while it ran, inside the open.
    pub reads: u64,
    pub vm_steps: u64,
    pub full_scans: u64,
    /// Page reads run alone and cold, if it only reads.
    pub cold: Option<u64>,
    /// Its query plan (`EXPLAIN QUERY PLAN` details).
    pub plan: Vec<String>,
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
    /// Whether its plan scans the events table itself, not one of its
    /// indexes: the audit history is the store's largest table, and startup
    /// is meant to reach it only through an index.
    pub fn scans_events(&self) -> bool {
        self.plan
            .iter()
            .any(|step| step == "SCAN events" || step.starts_with("SCAN events "))
            && !self
                .plan
                .iter()
                .any(|step| step.starts_with("SCAN events USING"))
    }
    /// The first line of its text, shortened, for a report.
    pub fn short(&self) -> String {
        let text: String = self.sql.split_whitespace().collect::<Vec<_>>().join(" ");
        text.chars().take(100).collect()
    }
}

/// What core's open on one copy executed.
#[derive(Clone, Debug, Default)]
pub struct Trace {
    pub statements: Vec<Statement>,
    /// Page reads outside every statement: preparing, the schema.
    pub outside: u64,
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
}

/// What a child process measured.
struct Child {
    opened: Opened,
    trace: Option<Trace>,
    refused: Option<String>,
}

const MEASURE: &str = "LIMEOS_SOAK_MEASURE";

/// Runs `measure_in_child` in a new process of this binary on `path`.
fn in_child(store: &str, path: &Path, now: i64) -> Child {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "soak::startup::measure_in_child",
            "--exact",
            "--ignored",
            "--nocapture",
            "--test-threads",
            "1",
        ])
        .env(MEASURE, format!("{store}\n{now}\n{}", path.display()))
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
                full_scans: number(&s["scans"]),
                cold: s["cold"].as_u64(),
                plan: s["plan"]
                    .as_array()
                    .map(|p| {
                        p.iter()
                            .filter_map(|x| x.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default(),
            })
            .collect(),
        outside: number(&value["outside"]),
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
    /// Statements running, innermost last.
    running: Vec<String>,
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
            Some(sql) => self.statements.get_mut(sql).unwrap().0.reads += reads,
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
        match event {
            TraceEvent::Stmt(stmt, _) => {
                let sql = stmt.sql().into_owned();
                if tracer.running.last() != Some(&sql) {
                    tracer.running.push(sql.clone());
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
                if let Some(at) = tracer.running.iter().rposition(|s| *s == sql) {
                    tracer.running.remove(at);
                }
                if let Some((statement, _)) = tracer.statements.get_mut(&sql) {
                    statement.runs += 1;
                    let status = |s| u64::try_from(stmt.get_status(s)).unwrap_or(0);
                    statement.vm_steps = statement.vm_steps.max(status(StatementStatus::VmStep));
                    statement.full_scans = statement
                        .full_scans
                        .max(status(StatementStatus::FullscanStep));
                }
            }
            _ => {}
        }
    });
}
fn hook(conn: &rusqlite::Connection) {
    conn.trace_v2(
        TraceEventCodes::SQLITE_TRACE_STMT | TraceEventCodes::SQLITE_TRACE_PROFILE,
        Some(traced),
    );
}

/// Each statement run alone and cold on its own connection to `path` (if it
/// only reads), and its plan.
fn cold(path: &Path, statements: &mut [(Statement, Option<String>)]) {
    let reads = Reads::open();
    for (statement, expanded) in statements {
        let sql = expanded.clone().unwrap_or_else(|| statement.sql.clone());
        let conn = rusqlite::Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .expect("a side connection to the copy");
        statement.plan = conn
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .and_then(|mut s| s.query_map([], |r| r.get::<_, String>(3))?.collect())
            .unwrap_or_default();
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

/// The child half of `in_child`: a no-op unless run by it.
#[test]
#[ignore = "the child half of an isolated startup measurement, run by the soak itself"]
fn measure_in_child() {
    let Some(request) = std::env::var_os(MEASURE) else {
        return;
    };
    let request = request.into_string().unwrap();
    let mut parts = request.splitn(3, '\n');
    let (store, now, path) = (
        parts.next().unwrap(),
        parts.next().unwrap().parse::<i64>().unwrap(),
        PathBuf::from(parts.next().unwrap()),
    );
    let result = if store == "core" {
        // A second copy, for each statement run cold: the open writes the
        // first (recovery, the generation).
        let second = path.with_extension("cold.sqlite");
        std::fs::copy(&path, &second).unwrap();
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
        let total = statements.iter().map(|(s, _)| s.reads).sum::<u64>() + tracer.outside;
        json!({
            "syscr": total,
            "rchar": rchar,
            "micros": elapsed.as_micros() as u64,
            "refused": opened.err().map(|e| format!("{e:?}")),
            "outside": tracer.outside,
            "statements": statements.iter().map(|(s, _)| json!({
                "sql": s.sql,
                "runs": s.runs,
                "reads": s.reads,
                "vm": s.vm_steps,
                "scans": s.full_scans,
                "cold": s.cold,
                "plan": s.plan,
            })).collect::<Vec<_>>(),
        })
    } else {
        let dir = path.parent().unwrap().to_path_buf();
        let (io, at) = (Io::now(), Instant::now());
        let opened = ReceiptStore::open_at(&dir, now).map(drop);
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
