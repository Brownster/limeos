//! Startup measured on a consistent copy of a store, in a unit that does not
//! depend on the filesystem or the machine.
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
//! A second scan of a table larger than the cache (2 MiB) is counted in
//! full; one of a smaller table is served from the cache and costs CPU, not
//! I/O, and is not seen.
//!
//! Two other units were tried and rejected. A copy whose header shrinks the
//! page cache to its floor (`PRAGMA default_cache_size`) counts every fetch,
//! but the count jumps by up to four times between two nights' nearly equal
//! files: the foreign-key check probes the jobs index once per event, and
//! whether those probes stay cached flips with the index's depth. SQLite's
//! own counters (`sqlite3_db_status` cache hits and misses,
//! `sqlite3_stmt_status`, a progress handler) belong to a connection, and the
//! connection `Store::open` opens is private to the store: reaching it needs
//! `unsafe` (forbidden workspace-wide) or a production seam, a `test-support`
//! hook on the connection before startup runs, left for the next round.
use crate::{
    executor::Opened,
    sim::{Sim, Startup, sizes},
    support::Io,
};
use limeos_executor_container::ReceiptStore;
use limeos_persistence::Store;
use rusqlite::MAIN_DB;
use std::{
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
    time::Instant,
};

/// `source`'s committed content (database and WAL) as a single file at
/// `target`.
pub fn copy(source: &Path, target: &Path) {
    let from = crate::checks::reader(source).expect("side connection");
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

/// The page fetches, bytes read and wall time of `open`, on this thread.
pub fn measure<T>(open: impl FnOnce() -> T) -> (T, Opened) {
    let (io, at) = (Io::now(), Instant::now());
    let value = open();
    let opened = Opened {
        io: Io::now().since(io),
        elapsed: at.elapsed(),
    };
    (value, opened)
}

/// `Store::open` on a copy of the live core store, measured alone in a
/// child process with startup's known statements (`probe`); the copy is then
/// deleted. Startup checks run as at a restart, recovery included.
pub fn core(sim: &mut Sim) {
    let dir = scratch(sim, "startup-core");
    let path = dir.join("core.sqlite");
    copy(&sim.core_path, &path);
    let (file, wal) = sizes(&path);
    let child = in_child("core", &path, sim.now());
    if let Some(refused) = &child.refused {
        sim.violation(
            crate::checks::STARTUP,
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
        probe: child.probe,
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
            crate::checks::STARTUP,
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
        probe: None,
    });
}

/// What a child process measured.
struct Child {
    opened: Opened,
    probe: Option<Probe>,
    refused: Option<String>,
}

const MEASURE: &str = "LIMEOS_SOAK_MEASURE";

/// Runs `measure_in_child` in a new process of this binary on `path`.
fn in_child(store: &str, path: &Path, now: i64) -> Child {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "startup::measure_in_child",
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
    let fields: Vec<&str> = line.split(' ').collect();
    let number = |i: usize| -> u64 { fields[i].parse().unwrap() };
    Child {
        opened: Opened {
            io: Io {
                rchar: number(1),
                syscr: number(0),
                ..Io::default()
            },
            elapsed: std::time::Duration::from_micros(number(2)),
        },
        probe: (store == "core").then(|| Probe {
            known: number(3),
            proof: number(4),
            proof_by_kind: number(5) == 1,
        }),
        refused: (fields[6] != "-").then(|| fields[6].to_owned()),
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
    let probe = if store == "core" {
        probe(&path)
    } else {
        Probe::default()
    };
    let (refused, opened) = if store == "core" {
        let (result, opened) = measure(|| Store::open(&path).map(drop));
        (result.err(), opened)
    } else {
        let dir = path.parent().unwrap().to_path_buf();
        let (result, opened) = measure(|| ReceiptStore::open_at(&dir, now).map(drop));
        (result.err(), opened)
    };
    println!(
        "MEASURED {} {} {} {} {} {} {}",
        opened.io.syscr,
        opened.io.rchar,
        opened.elapsed.as_micros(),
        probe.known,
        probe.proof,
        u8::from(probe.proof_by_kind),
        refused.map_or("-".into(), |e| format!("{e:?}").replace(' ', ""))
    );
}

/// The reason a measured open was on a copy (the deterministic series).
pub const COPY: &str = "measured on a copy";

/// SQLite page reads of an open measured on a copy.
pub fn pages(start: &Startup) -> u64 {
    start.opened.io.syscr
}

/// The storage-mount proof check of `storage_mounts::check`, verbatim.
pub const MOUNT_PROOF: &str = "SELECT EXISTS(SELECT 1 FROM jobs j WHERE json_extract(j.intent,'$.operation')='storage_mount' AND (j.state='succeeded' OR (j.state='precondition_changed' AND EXISTS(SELECT 1 FROM events e WHERE e.job=j.id AND json_extract(e.kind,'$.kind')='job_transition' AND json_extract(e.kind,'$.state')='running'))) AND NOT EXISTS(SELECT 1 FROM storage_mount_results r WHERE r.job=j.id))";

/// Every statement of `Store::open`'s startup checks that reads more than a
/// few pages of a store without automation, storage or backup rows, copied
/// verbatim from `crates/persistence/src` (the file is named with each).
/// Each measured cold, on its own fresh connection, reads at least as many
/// pages as it does inside the open, where the cache is shared (an LRU
/// cache that starts full misses no more than one that starts empty), so
/// the open can read more than their sum only by running something else:
/// an extra scan, or a query whose index went missing. A statement added to
/// the startup checks must be added here, as a dispatcher change must be
/// mirrored in `dispatch.rs`.
const KNOWN: [(&str, &str); 11] = [
    ("quick_check (lib.rs)", "PRAGMA quick_check"),
    (
        "lock, resource and resolution invariants with the foreign-key check (resources.rs)",
        "SELECT EXISTS(SELECT 1 FROM jobs j WHERE NOT EXISTS(SELECT 1 FROM job_resources r WHERE r.job=j.id AND r.resource=j.resource) OR (SELECT count(*) FROM job_resources r WHERE r.job=j.id)>128) OR EXISTS(SELECT 1 FROM job_resources r JOIN jobs j ON j.id=r.job LEFT JOIN resource_locks l ON l.job=r.job AND l.resource=r.resource WHERE j.state IN ('running','verifying','outcome_unknown','needs_intervention') AND l.resource IS NULL) OR EXISTS(SELECT 1 FROM resource_locks l JOIN jobs j ON j.id=l.job WHERE j.state NOT IN ('running','verifying','outcome_unknown','needs_intervention')) OR EXISTS(SELECT 1 FROM jobs j WHERE j.state='resolved' AND NOT EXISTS(SELECT 1 FROM job_resolutions r WHERE r.job=j.id AND r.confirmation IS NOT NULL)) OR EXISTS(SELECT 1 FROM job_resolutions r JOIN jobs j ON j.id=r.job WHERE (r.confirmation IS NOT NULL) <> (j.state='resolved')) OR EXISTS(SELECT 1 FROM pragma_foreign_key_check)",
    ),
    (
        "storage target jobs (resources.rs)",
        "SELECT count(*) FROM (SELECT id,principal,intent,digest,resource,revision,deadline FROM jobs WHERE json_extract(intent,'$.operation')='storage_prepare_targets')",
    ),
    (
        "storage mount jobs (storage_mounts.rs)",
        "SELECT count(*) FROM (SELECT id,principal,intent,digest,resource,revision,deadline FROM jobs WHERE json_extract(intent,'$.operation')='storage_mount')",
    ),
    ("storage mount proofs (storage_mounts.rs)", MOUNT_PROOF),
    (
        "unlinked backup preparations (backup_preparation.rs)",
        "SELECT EXISTS(SELECT 1 FROM jobs j WHERE json_extract(j.intent,'$.operation')='backup_prepare_private' AND NOT EXISTS(SELECT 1 FROM backup_preparation_plans p WHERE p.job=j.id))",
    ),
    (
        "occurrences of other jobs (automation.rs)",
        "SELECT EXISTS(SELECT 1 FROM automation_occurrences o JOIN automation_schedules s ON s.id=o.schedule WHERE o.owner<>s.owner OR o.policy<>s.policy OR o.revision>s.revision OR (o.revision=s.revision AND (o.owner_revision<>s.owner_revision OR s.settled_through IS NULL OR o.due>s.settled_through))) OR EXISTS(SELECT 1 FROM automation_occurrences o JOIN jobs j ON j.id=o.job WHERE json_extract(j.intent,'$.operation') IS NOT 'automation_observe_health')",
    ),
    (
        "observation jobs (automation.rs)",
        "SELECT count(*) FROM (SELECT j.id,j.principal,j.idempotency,j.intent,j.digest,j.resource,j.state,j.revision,j.deadline,o.schedule,o.revision,o.due,o.owner,o.owner_revision,o.created,o.policy_revision FROM jobs j LEFT JOIN automation_occurrences o ON o.job=j.id WHERE json_extract(j.intent,'$.operation')='automation_observe_health')",
    ),
    (
        "observation results and notifications (automation/completion/check.rs)",
        "SELECT EXISTS(SELECT 1 FROM jobs j WHERE json_extract(j.intent,'$.operation')='automation_observe_health' AND j.state IN ('succeeded','failed','precondition_changed') AND NOT EXISTS(SELECT 1 FROM automation_observation_results r WHERE r.job=j.id)) OR EXISTS(SELECT 1 FROM automation_observation_results r LEFT JOIN automation_streams s ON s.owner=r.owner LEFT JOIN jobs j ON j.id=r.job LEFT JOIN users u ON u.id=r.owner WHERE s.owner IS NULL OR j.id IS NULL OR u.id IS NULL) OR EXISTS(SELECT 1 FROM events WHERE json_extract(kind,'$.kind')='automation_notification_created' GROUP BY json_extract(kind,'$.notification') HAVING count(*)<>1)",
    ),
    (
        "audit reconciliation (retention.rs)",
        "SELECT audit_bytes,(SELECT count(*) FROM events) FROM meta WHERE id=1",
    ),
    (
        "interrupted jobs (lib.rs)",
        "SELECT count(*) FROM jobs WHERE state IN ('running','verifying')",
    ),
];

/// The cold page reads of startup's known statements on an unopened copy,
/// and of the storage-mount proof check among them with its plan.
#[derive(Clone, Copy, Debug, Default)]
pub struct Probe {
    /// The sum over `KNOWN`.
    pub known: u64,
    /// The storage-mount proof check alone.
    pub proof: u64,
    /// Whether its plan looks the job's events up by kind
    /// (`events_kind`) rather than by job (`events_job`).
    pub proof_by_kind: bool,
}

pub fn probe(path: &Path) -> Probe {
    let mut probe = Probe::default();
    for (name, sql) in KNOWN {
        let conn = crate::checks::reader(path).expect("side connection");
        let before = Io::now();
        let _: rusqlite::types::Value = conn
            .query_row(sql, [], |r| r.get(0))
            .unwrap_or_else(|e| panic!("startup statement {name}: {e}"));
        let pages = Io::now().since(before).syscr;
        probe.known += pages;
        if sql == MOUNT_PROOF {
            probe.proof = pages;
            let plan: Vec<String> = conn
                .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
                .and_then(|mut s| s.query_map([], |r| r.get::<_, String>(3))?.collect())
                .unwrap();
            probe.proof_by_kind = plan
                .iter()
                .any(|step| step.starts_with("SEARCH e USING INDEX events_kind"));
        }
    }
    probe
}
