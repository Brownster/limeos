use super::snapshot::{Kind, classify, snapshot_bytes, snapshot_path, state_path};
use super::*;
use std::os::unix::fs::{MetadataExt, PermissionsExt};

/// Every schema object, as `(type, name, table, sql)`, in a fixed order.
type Dump = Vec<(String, String, String, Option<String>)>;
fn dump(conn: &Connection) -> Dump {
    conn.prepare("SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY type,name")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap()
}
fn user_version(path: &Path) -> u32 {
    Connection::open(path)
        .unwrap()
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap()
}
/// A store at schema `version` as the build of that version left it: its
/// migrations run in a WAL database, with no core lock taken since.
fn store_at(dir: &Path, version: u32) -> PathBuf {
    let path = dir.join("core.sqlite");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL").unwrap();
    for migration in &MIGRATIONS[..version as usize] {
        conn.execute_batch(migration.sql).unwrap();
    }
    assert_eq!(
        conn.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        version
    );
    path
}
/// The store's snapshot files in `dir` (finished, records and leftovers),
/// by name.
fn snapshots(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|name| classify("core.sqlite", name).is_some())
        .collect();
    names.sort();
    names
}
fn kept(from: u32) -> [String; 2] {
    [
        format!("core.sqlite.v{from}.snapshot"),
        format!("core.sqlite.v{from}.snapshot.state"),
    ]
}
/// What the snapshot code removed and wrote since the last call, in order.
fn events() -> Vec<String> {
    super::snapshot::EVENTS.with(|events| std::mem::take(&mut *events.borrow_mut()))
}
fn writes(events: &[String]) -> usize {
    events.iter().filter(|e| e.starts_with("write ")).count()
}
/// Bytes the store's directory holds, as the simulated disk counts them,
/// leaving out the WAL and its index (which SQLite removes on close).
fn allocated(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            !name.ends_with("-wal") && !name.ends_with("-shm")
        })
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.blocks() * 512)
        .sum()
}
fn listing(dir: &Path) -> Vec<(String, u64, u64)> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| {
            let meta = e.metadata().unwrap();
            (
                e.file_name().to_string_lossy().into_owned(),
                meta.len(),
                meta.blocks() * 512,
            )
        })
        .collect();
    files.sort();
    files
}
/// The migration table with one more entry after it, which forgets to set
/// its version: every upgrade to it fails after the snapshot is in place.
fn failing() -> Vec<Migration> {
    let mut extended = MIGRATIONS.to_vec();
    extended.push(Migration {
        version: SCHEMA_VERSION + 1,
        description: "test only: forgets its version",
        sql: "CREATE TABLE probe (id INTEGER PRIMARY KEY) STRICT;",
        checked: &[],
        rewrites: &[],
    });
    extended
}
fn budget_rows(file: &Path, id: &str) -> i64 {
    Connection::open_with_flags(file, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .unwrap()
        .query_row("SELECT count(*) FROM budgets WHERE id=?", [id], |r| {
            r.get(0)
        })
        .unwrap()
}

#[test]
fn the_table_is_strictly_ordered_and_contiguous() {
    // An empty table does not compile: `SCHEMA_VERSION` is its last entry.
    for (index, migration) in MIGRATIONS.iter().enumerate() {
        assert_eq!(migration.version as usize, index + 1, "entry {index}");
        // Each migration sets its own version, as its last statement.
        let last = migration
            .sql
            .trim_end()
            .lines()
            .last()
            .unwrap_or_default()
            .trim();
        assert_eq!(
            last,
            format!("PRAGMA user_version={};", migration.version),
            "version {}",
            migration.version
        );
        assert!(!migration.description.trim().is_empty());
    }
    assert_eq!(SCHEMA_VERSION, MIGRATIONS.last().unwrap().version);
    assert!(
        MIGRATIONS[INDEXED_SCHEMA as usize - 1]
            .sql
            .contains("CREATE INDEX events_job")
    );
    // What the entries name exists in the schema they build.
    let reference = reference(MIGRATIONS).unwrap();
    for object in MIGRATIONS.iter().flat_map(|m| m.checked) {
        assert!(
            stored_sql(&reference, object.kind(), object.name())
                .unwrap()
                .is_some(),
            "{object:?}"
        );
    }
}

/// The data path is unchanged by the table: a store at any older version
/// migrates to exactly the schema a new store gets, and each upgrade leaves
/// one snapshot and its record, at the version it started from, private to
/// the store's owner and openable.
#[test]
fn every_older_version_migrates_to_the_schema_of_a_new_store() {
    let fresh = tempfile::tempdir().unwrap();
    let expected = dump(&Store::open(&fresh.path().join("core.sqlite")).unwrap().conn);
    assert!(snapshots(fresh.path()).is_empty(), "a new store has none");
    for from in 1..SCHEMA_VERSION {
        let dir = tempfile::tempdir().unwrap();
        let path = store_at(dir.path(), from);
        let store = Store::open(&path).unwrap();
        assert_eq!(dump(&store.conn), expected, "from version {from}");
        drop(store);
        assert_eq!(user_version(&path), SCHEMA_VERSION);
        assert_eq!(snapshots(dir.path()), kept(from), "from version {from}");
        for file in [snapshot_path(&path, from), state_path(&path, from)] {
            let mode = std::fs::metadata(file.unwrap())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "from version {from}");
        }
        assert_eq!(user_version(&snapshot_path(&path, from).unwrap()), from);
    }
}

/// The data steps run too: a version-1 store with users, jobs and events
/// gets its resource sets and locks (v7), settlement times (v19) and its
/// recovery, and still the schema of a new store.
#[test]
fn a_version_1_store_with_data_migrates_its_rows() {
    let fresh = tempfile::tempdir().unwrap();
    let expected = dump(&Store::open(&fresh.path().join("core.sqlite")).unwrap().conn);
    let dir = tempfile::tempdir().unwrap();
    let path = store_at(dir.path(), 1);
    let id = |n: char| n.to_string().repeat(64);
    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "INSERT INTO users VALUES ('alice','alice','$argon2id$fixture','admin',1)",
        [],
    )
    .unwrap();
    for (job, state, resource) in [
        (id('a'), "queued", "container:a"),
        (id('b'), "running", "container:b"),
        (id('c'), "succeeded", "container:c"),
    ] {
        conn.execute(
            "INSERT INTO jobs VALUES (?,'alice',?,'{\"operation\":\"container_restart\"}',?,?,?,1,1,4000000000)",
            params![job, format!("key-{state}"), "d".repeat(64), resource, state],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events(principal,job,kind,created) VALUES ('alice',?,'{\"kind\":\"session_created\"}',100)",
            [&job],
        )
        .unwrap();
    }
    drop(conn);
    let store = Store::open(&path).unwrap();
    assert_eq!(dump(&store.conn), expected);
    let rows = |sql: &str| -> Vec<(String, Option<String>)> {
        store
            .conn
            .prepare(sql)
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap()
    };
    assert_eq!(
        rows("SELECT job,resource FROM job_resources ORDER BY job"),
        [
            (id('a'), Some("container:a".into())),
            (id('b'), Some("container:b".into())),
            (id('c'), Some("container:c".into())),
        ]
    );
    assert_eq!(
        rows("SELECT job,resource FROM resource_locks"),
        [(id('b'), Some("container:b".into()))]
    );
    assert_eq!(
        rows("SELECT id,state||':'||(settled IS NOT NULL) FROM jobs ORDER BY id"),
        [
            (id('a'), Some("queued:0".into())),
            (id('b'), Some("needs_intervention:0".into())),
            (id('c'), Some("succeeded:1".into())),
        ]
    );
    let events: i64 = store
        .conn
        .query_row("SELECT count(*) FROM events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(events, 4, "three kept and one recovery record");
}

/// The plan's exit test: one more entry, added to the table and nowhere
/// else, is applied to an existing store (after a snapshot) and to a new
/// one, and the startup check then holds the store to what it declared.
#[test]
fn a_migration_added_to_the_table_alone_is_applied_and_checked() {
    let next = SCHEMA_VERSION + 1;
    let sql: &'static str = Box::leak(
        format!(
            "CREATE TABLE probe (id INTEGER PRIMARY KEY, note TEXT NOT NULL CHECK(length(note)<=8)) STRICT;
CREATE TRIGGER probe_kept BEFORE DELETE ON probe BEGIN SELECT RAISE(ABORT,'kept'); END;
INSERT INTO probe(note) SELECT 'migrated' FROM meta;
PRAGMA user_version={next};"
        )
        .into_boxed_str(),
    );
    let mut extended = MIGRATIONS.to_vec();
    extended.push(Migration {
        version: next,
        description: "test only: a probe table",
        sql,
        checked: &[Object::Table("probe")],
        rewrites: &[],
    });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    let mut store = Store::open(&path).unwrap();
    store.issue_bootstrap("token", 100).unwrap();
    drop(store);
    // Applied, after a snapshot of the store as it was.
    let store = Store::open_migrating(&path, &extended).unwrap();
    let note: String = store
        .conn
        .query_row("SELECT note FROM probe", [], |r| r.get(0))
        .unwrap();
    assert_eq!(note, "migrated");
    drop(store);
    assert_eq!(user_version(&path), next);
    assert_eq!(snapshots(dir.path()), kept(SCHEMA_VERSION));
    Store::open_migrating(&path, &extended).unwrap();
    // The build without it refuses the newer store.
    assert_eq!(Store::open(&path).err().unwrap().0, ErrorCode::Conflict);
    // Checked: its table's SQL and its trigger, as declared.
    let tamper = |sql: &str| {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(sql).unwrap();
    };
    tamper(
        "PRAGMA writable_schema=ON; UPDATE sqlite_schema SET sql=replace(sql,'<=8','<=9') WHERE name='probe'; PRAGMA writable_schema=RESET;",
    );
    assert_eq!(
        Store::open_migrating(&path, &extended).err().unwrap().0,
        ErrorCode::StateNotDurable
    );
    assert_eq!(
        crate::schema_mismatch(&path),
        Some(SchemaMismatch {
            kind: "table",
            name: "probe".into()
        })
    );
    tamper(
        "PRAGMA writable_schema=ON; UPDATE sqlite_schema SET sql=replace(sql,'<=9','<=8') WHERE name='probe'; PRAGMA writable_schema=RESET;",
    );
    Store::open_migrating(&path, &extended).unwrap();
    tamper("DROP TRIGGER probe_kept");
    assert_eq!(
        Store::open_migrating(&path, &extended).err().unwrap().0,
        ErrorCode::StateNotDurable
    );
    assert_eq!(
        crate::schema_mismatch(&path),
        Some(SchemaMismatch {
            kind: "trigger",
            name: "probe_kept".into()
        })
    );
    // A new store gets it too, with no snapshot.
    let fresh = tempfile::tempdir().unwrap();
    let store = Store::open_migrating(&fresh.path().join("core.sqlite"), &extended).unwrap();
    assert_eq!(
        store
            .conn
            .pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        next
    );
    drop(store);
    assert!(snapshots(fresh.path()).is_empty());
}

/// A migration that does not set the version it is listed at is refused,
/// and the store is left as it was.
#[test]
fn a_migration_that_does_not_reach_its_version_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    drop(Store::open(&path).unwrap());
    assert_eq!(
        Store::open_migrating(&path, &failing()).err().unwrap().0,
        ErrorCode::StateNotDurable
    );
    assert_eq!(user_version(&path), SCHEMA_VERSION);
}

/// Version 16 widens the `jobs` state CHECK by editing its text; when the
/// text is not what it expects, its `replace` changes nothing and SQLite
/// reports no error. Before PR-23 such a store reached version 19 with no
/// `resolved` state, so no stuck job could ever be resolved. Version 16's
/// `rewrites` refuses it right after that step and names the table; remove
/// it and this fails (the final check would refuse too, after v17-v19).
#[test]
fn a_jobs_table_that_version_16_could_not_widen_refuses_the_upgrade() {
    let dir = tempfile::tempdir().unwrap();
    let path = store_at(dir.path(), 15);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "PRAGMA writable_schema=ON;
         UPDATE sqlite_schema SET sql=replace(sql,'''needs_intervention'')','''needs_intervention'' )') WHERE type='table' AND name='jobs';
         PRAGMA writable_schema=RESET;",
    )
    .unwrap();
    drop(conn);
    assert_eq!(
        Store::open(&path).err().unwrap().0,
        ErrorCode::StateNotDurable
    );
    // Refused right after version 16, before versions 17 to 19 ran (and
    // wrote their pages) inside the transaction that is rolled back.
    crate::APPLIED.with(|applied| assert_eq!(*applied.borrow(), [16]));
    assert_eq!(
        crate::schema_mismatch(&path),
        Some(SchemaMismatch {
            kind: "table",
            name: "jobs".into()
        })
    );
    assert_eq!(crate::schema_mismatch(&path), None, "reported once");
    assert_eq!(user_version(&path), 15);
    // Version 16's SQL alone runs without error and leaves the CHECK as it
    // was: what the comparison is for.
    let mut conn = Connection::open(&path).unwrap();
    let tx = conn.transaction().unwrap();
    tx.execute_batch(MIGRATIONS[15].sql).unwrap();
    let jobs = stored_sql(&tx, "table", "jobs").unwrap().unwrap();
    assert!(!jobs.contains("'resolved'"), "{jobs}");
}

/// A current store whose `jobs` table lost the `resolved` state (or
/// changed in any other way) is refused at startup, and the table named.
#[test]
fn a_changed_jobs_table_refuses_startup_and_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    drop(Store::open(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "PRAGMA writable_schema=ON;
         UPDATE sqlite_schema SET sql=replace(sql,',''resolved'')',')') WHERE type='table' AND name='jobs';
         PRAGMA writable_schema=RESET;",
    )
    .unwrap();
    drop(conn);
    assert_eq!(
        Store::open(&path).err().unwrap().0,
        ErrorCode::StateNotDurable
    );
    assert_eq!(
        crate::schema_mismatch(&path),
        Some(SchemaMismatch {
            kind: "table",
            name: "jobs".into()
        })
    );
    assert!(
        snapshots(dir.path()).is_empty(),
        "a current store takes none"
    );
}

/// Committed rows still in the WAL are in the snapshot; a copy of the
/// database file alone would not hold them.
#[test]
fn the_snapshot_holds_what_is_still_in_the_wal() {
    let dir = tempfile::tempdir().unwrap();
    let path = store_at(dir.path(), SCHEMA_VERSION - 1);
    let conn = Connection::open(&path).unwrap();
    conn.set_db_config(
        rusqlite::config::DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE,
        true,
    )
    .unwrap();
    conn.execute_batch(
        "PRAGMA wal_autocheckpoint=0; INSERT INTO budgets(id,limit_units) VALUES ('in-wal',7);",
    )
    .unwrap();
    drop(conn);
    assert!(
        std::fs::metadata(path.with_extension("sqlite-wal"))
            .unwrap()
            .len()
            > 0
    );
    let bare = dir.path().join("bare.sqlite");
    std::fs::copy(&path, &bare).unwrap();
    assert_eq!(budget_rows(&bare, "in-wal"), 0, "the file alone misses it");
    drop(Store::open(&path).unwrap());
    let snapshot = snapshot_path(&path, SCHEMA_VERSION - 1).unwrap();
    // Restored as the operator would: copied over the store's name in an
    // empty directory, where the current build opens and migrates it.
    let restored = tempfile::tempdir().unwrap();
    let store = restored.path().join("core.sqlite");
    std::fs::copy(&snapshot, &store).unwrap();
    assert_eq!(budget_rows(&store, "in-wal"), 1);
    drop(Store::open(&store).unwrap());
    assert_eq!(user_version(&store), SCHEMA_VERSION);
}

/// A snapshot whose copy, sync or rename fails stops the upgrade: the store
/// keeps its version, no partial file is left, and the previous snapshot
/// (which the free space did not need) is still there.
#[test]
fn a_failed_snapshot_write_stops_the_migration_and_keeps_the_previous_one() {
    let from = SCHEMA_VERSION - 1;
    for step in ["vacuum", "sync", "rename"] {
        let dir = tempfile::tempdir().unwrap();
        let path = store_at(dir.path(), from);
        std::fs::write(dir.path().join("core.sqlite.v12.snapshot"), b"old").unwrap();
        events();
        super::snapshot::FAIL_AT.set(Some(step));
        let opened = Store::open(&path);
        super::snapshot::FAIL_AT.set(None);
        assert_eq!(
            opened.err().unwrap().0,
            ErrorCode::StateNotDurable,
            "{step}"
        );
        assert_eq!(
            events(),
            [format!("write core.sqlite.v{from}.snapshot")],
            "{step}"
        );
        assert_eq!(user_version(&path), from, "{step}");
        assert_eq!(
            snapshots(dir.path()),
            ["core.sqlite.v12.snapshot"],
            "{step}"
        );
        drop(Store::open(&path).unwrap());
        assert_eq!(snapshots(dir.path()), kept(from), "{step}");
    }
}

/// A record that cannot be written leaves a snapshot the next attempt cannot
/// keep: noted for the daemon to log, and taken (and recorded) again then.
#[test]
fn a_snapshot_without_its_record_is_noted_and_taken_again() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    drop(Store::open(&path).unwrap());
    let extended = failing();
    events();
    super::snapshot::FAIL_AT.set(Some("record"));
    assert!(Store::open_migrating(&path, &extended).is_err());
    super::snapshot::FAIL_AT.set(None);
    assert!(crate::unrecorded_snapshot(&path));
    assert!(!crate::unrecorded_snapshot(&path), "reported once");
    assert_eq!(
        snapshots(dir.path()),
        [format!("core.sqlite.v{SCHEMA_VERSION}.snapshot")]
    );
    assert_eq!(writes(&events()), 1);
    assert!(Store::open_migrating(&path, &extended).is_err());
    assert_eq!(writes(&events()), 1, "taken again");
    assert!(!crate::unrecorded_snapshot(&path));
    assert_eq!(snapshots(dir.path()), kept(SCHEMA_VERSION));
    assert!(Store::open_migrating(&path, &extended).is_err());
    assert_eq!(writes(&events()), 0, "then kept");
}

/// Short of space for the snapshot and the upgrade, the open is refused
/// before writing anything; `upgrade_space` then reports what the open
/// measured (once), and its own estimate agrees.
#[test]
fn a_snapshot_that_would_not_fit_refuses_and_says_how_much_it_needs() {
    let dir = tempfile::tempdir().unwrap();
    let from = SCHEMA_VERSION - 1;
    let path = store_at(dir.path(), from);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c WHERE x<5000) INSERT INTO budgets(id,limit_units) SELECT printf('budget-%d-%s',x,hex(randomblob(32))),x FROM c;",
    )
    .unwrap();
    let bytes = snapshot_bytes(&conn).unwrap();
    let (build, others) = upgrade_writes(&conn, from).unwrap();
    drop(conn);
    crate::OPEN_FREE_BYTES.set(Some(0));
    let needed = crate::upgrade_space(&path).unwrap().needed;
    assert_eq!(needed, bytes + upgrade_remaining(from, build, others));
    crate::OPEN_FREE_BYTES.set(Some(needed - 1));
    let short = Some(UpgradeSpace {
        from,
        needed,
        available: needed - 1,
    });
    assert_eq!(
        Store::open(&path).err().unwrap().0,
        ErrorCode::StateNotDurable
    );
    assert_eq!(crate::upgrade_space(&path), short, "as the open measured");
    assert_eq!(crate::upgrade_space(&path), short, "and estimated");
    assert_eq!(user_version(&path), from);
    assert!(snapshots(dir.path()).is_empty());
    crate::OPEN_FREE_BYTES.set(Some(needed));
    drop(Store::open(&path).unwrap());
    assert_eq!(crate::upgrade_space(&path), None);
    crate::OPEN_FREE_BYTES.set(None);
    let written = std::fs::metadata(snapshot_path(&path, from).unwrap())
        .unwrap()
        .len();
    assert!(written <= bytes, "estimated {bytes}, wrote {written}");
}

/// On a disk that holds the store, one more snapshot-sized file and exactly
/// enough besides for the new snapshot and the migrations (no slack), the
/// upgrade goes through when that file is a leftover of an interrupted
/// snapshot, a stale snapshot of the same version or an older version's:
/// a leftover is removed before the store is opened, a finished snapshot
/// before the new one is written (it is needed as space). With one byte
/// less it is refused and a finished snapshot is still there.
///
/// The simulated disk is consulted where `Store::open` measures free space;
/// it cannot make SQLite's own allocation of the WAL index fail, which is
/// why `leftovers_go_before_the_store_is_opened` checks that ordering.
#[test]
fn snapshot_files_that_go_count_as_free_space() {
    let from = SCHEMA_VERSION - 1;
    for stale in [
        format!("core.sqlite.v{from}.snapshot.partial"),
        format!("core.sqlite.v{from}.snapshot"),
        "core.sqlite.v12.snapshot".to_owned(),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = store_at(dir.path(), from);
        crate::OPEN_FREE_BYTES.set(Some(0));
        let needed = crate::upgrade_space(&path).unwrap().needed;
        crate::OPEN_FREE_BYTES.set(None);
        // The WAL index SQLite creates when it opens the store, as large as
        // the open will find it.
        let index = {
            let conn = Connection::open(&path).unwrap();
            conn.query_row("SELECT count(*) FROM sqlite_schema", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap();
            let mut shm = path.as_os_str().to_owned();
            shm.push("-shm");
            std::fs::metadata(PathBuf::from(shm)).unwrap().blocks() * 512
        };
        let capacity = allocated(dir.path()) + index + needed;
        // Whole blocks, so the file takes no more than its share of the
        // disk: the rest of `needed` is free space.
        let size = needed / 4096 * 4096;
        let leave = || {
            std::fs::write(dir.path().join(&stale), vec![7u8; size as usize]).unwrap();
        };
        leave();
        crate::OPEN_DISK_CAPACITY.set(Some(capacity - 1));
        events();
        let refused = Store::open(&path);
        assert_eq!(
            refused.err().map(|e| e.0),
            Some(ErrorCode::StateNotDurable),
            "{stale}: capacity {capacity}, index {index}, needed {needed}, {:?}",
            listing(dir.path())
        );
        assert_eq!(user_version(&path), from);
        if !stale.ends_with(".partial") {
            assert!(dir.path().join(&stale).exists(), "{stale} kept on refusal");
        }
        leave();
        crate::OPEN_DISK_CAPACITY.set(Some(capacity));
        events();
        let opened = Store::open(&path);
        crate::OPEN_DISK_CAPACITY.set(None);
        drop(opened.unwrap());
        let order = events();
        let removed = order.iter().position(|e| *e == format!("remove {stale}"));
        let written = order.iter().position(|e| e.starts_with("write "));
        assert!(removed.unwrap() < written.unwrap(), "{stale}: {order:?}");
        assert_eq!(user_version(&path), SCHEMA_VERSION, "{stale}");
        assert_eq!(snapshots(dir.path()), kept(from), "{stale}");
    }
}

/// With free space enough on its own, the new snapshot is written before
/// the previous one goes, so a failed write cannot leave none.
#[test]
fn the_previous_snapshot_goes_after_the_new_one_when_space_allows() {
    let from = SCHEMA_VERSION - 1;
    let dir = tempfile::tempdir().unwrap();
    let path = store_at(dir.path(), from);
    std::fs::write(dir.path().join("core.sqlite.v12.snapshot"), b"old").unwrap();
    std::fs::write(dir.path().join("core.sqlite.v12.snapshot.state"), b"{}").unwrap();
    events();
    drop(Store::open(&path).unwrap());
    let order = events();
    assert_eq!(
        order[0],
        format!("write core.sqlite.v{from}.snapshot"),
        "{order:?}"
    );
    assert!(
        order[1..]
            .iter()
            .all(|e| e.starts_with("remove core.sqlite.v12."))
    );
    assert_eq!(order.len(), 3);
}

/// Only what removing a file gives back counts as free: nothing for a file
/// another name still holds (a hard link), and only the allocated blocks of
/// a sparse one. Refused then, and the old snapshot keeps its name.
#[test]
fn only_space_that_removing_frees_is_counted() {
    let from = SCHEMA_VERSION - 1;
    for kind in ["linked", "sparse", "plain"] {
        let dir = tempfile::tempdir().unwrap();
        let path = store_at(dir.path(), from);
        crate::OPEN_FREE_BYTES.set(Some(0));
        let needed = crate::upgrade_space(&path).unwrap().needed;
        let old = dir.path().join("core.sqlite.v12.snapshot");
        match kind {
            "linked" => {
                let other = dir.path().join("kept-elsewhere");
                std::fs::write(&other, vec![7u8; needed as usize]).unwrap();
                std::fs::hard_link(&other, &old).unwrap();
            }
            "sparse" => File::create(&old).unwrap().set_len(needed).unwrap(),
            _ => std::fs::write(&old, vec![7u8; needed as usize]).unwrap(),
        }
        crate::OPEN_FREE_BYTES.set(Some(needed - 1));
        let opened = Store::open(&path);
        crate::OPEN_FREE_BYTES.set(None);
        if kind == "plain" {
            drop(opened.unwrap());
            assert!(!old.exists());
        } else {
            assert_eq!(
                opened.err().unwrap().0,
                ErrorCode::StateNotDurable,
                "{kind}"
            );
            assert!(old.exists(), "{kind}");
            assert_eq!(user_version(&path), from);
        }
    }
}

/// Leftovers of an interrupted snapshot go by plain file operations before
/// SQLite opens the store: even when the open then fails at once.
#[test]
fn leftovers_go_before_the_store_is_opened() {
    let leftover = |dir: &Path| {
        let file = dir.join("core.sqlite.v18.snapshot.partial");
        std::fs::write(&file, b"partial").unwrap();
        file
    };
    // A store from a newer build.
    let dir = tempfile::tempdir().unwrap();
    let path = store_at(dir.path(), 1);
    Connection::open(&path)
        .unwrap()
        .pragma_update(None, "user_version", 999)
        .unwrap();
    let file = leftover(dir.path());
    assert_eq!(Store::open(&path).err().unwrap().0, ErrorCode::Conflict);
    assert!(!file.exists());
    // Not a database at all.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    std::fs::write(&path, vec![1u8; 8192]).unwrap();
    let file = leftover(dir.path());
    assert!(Store::open(&path).is_err());
    assert!(!file.exists());
    // A leftover that cannot be removed refuses before the store exists.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    std::fs::create_dir(dir.path().join("core.sqlite.v18.snapshot.partial")).unwrap();
    assert_eq!(
        Store::open(&path).err().unwrap().0,
        ErrorCode::StateNotDurable
    );
    assert!(!path.exists(), "SQLite never opened it");
}

/// An upgrade that fails after its snapshot, started again and again (as
/// systemd does every two seconds), writes the snapshot once: the store is
/// unchanged, so the recorded snapshot is kept.
#[test]
fn a_failing_upgrade_started_again_writes_one_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    drop(Store::open(&path).unwrap());
    let extended = failing();
    events();
    assert!(Store::open_migrating(&path, &extended).is_err());
    assert_eq!(writes(&events()), 1);
    let snapshot = snapshot_path(&path, SCHEMA_VERSION).unwrap();
    let inode = std::fs::metadata(&snapshot).unwrap().ino();
    for _ in 0..5 {
        assert_eq!(
            Store::open_migrating(&path, &extended).err().unwrap().0,
            ErrorCode::StateNotDurable
        );
    }
    assert_eq!(events(), Vec::<String>::new(), "nothing removed or written");
    assert_eq!(std::fs::metadata(&snapshot).unwrap().ino(), inode);
    assert_eq!(snapshots(dir.path()), kept(SCHEMA_VERSION));
    assert_eq!(user_version(&path), SCHEMA_VERSION);
}

/// The snapshot is taken again whenever it cannot be shown to match the
/// store: after a commit (checkpointed, or only in the WAL), and with its
/// record missing, unreadable or for another version, its modification time
/// changed (ruled out without reading) or its bytes changed.
#[test]
fn a_snapshot_that_may_not_match_is_taken_again() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    drop(Store::open(&path).unwrap());
    let extended = failing();
    let (snapshot, state) = (
        snapshot_path(&path, SCHEMA_VERSION).unwrap(),
        state_path(&path, SCHEMA_VERSION).unwrap(),
    );
    let attempt = || {
        events();
        assert!(Store::open_migrating(&path, &extended).is_err());
        writes(&events()) == 1
    };
    assert!(attempt(), "first");
    assert!(!attempt(), "unchanged: kept");
    // A commit, checkpointed when the writer closes.
    Connection::open(&path)
        .unwrap()
        .execute("INSERT INTO budgets(id,limit_units) VALUES ('later',1)", [])
        .unwrap();
    assert!(attempt(), "after a commit");
    assert_eq!(budget_rows(&snapshot, "later"), 1);
    // A commit left only in the WAL.
    let conn = Connection::open(&path).unwrap();
    conn.set_db_config(
        rusqlite::config::DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE,
        true,
    )
    .unwrap();
    conn.execute_batch(
        "PRAGMA wal_autocheckpoint=0; INSERT INTO budgets(id,limit_units) VALUES ('in-wal',1);",
    )
    .unwrap();
    drop(conn);
    assert!(attempt(), "after a commit only in the WAL");
    assert_eq!(budget_rows(&snapshot, "in-wal"), 1);
    assert!(!attempt(), "unchanged again: kept");
    // The record.
    std::fs::remove_file(&state).unwrap();
    assert!(attempt(), "no record");
    std::fs::write(&state, b"not json").unwrap();
    assert!(attempt(), "unreadable record");
    let recorded = std::fs::read_to_string(&state).unwrap();
    std::fs::write(
        &state,
        recorded.replace(&format!("\"from\":{SCHEMA_VERSION}"), "\"from\":3"),
    )
    .unwrap();
    assert!(attempt(), "record for another version");
    // The snapshot's modification time, then its bytes (in place, keeping
    // its length, inode and modification time).
    File::options()
        .write(true)
        .open(&snapshot)
        .unwrap()
        .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1_000_000))
        .unwrap();
    assert!(attempt(), "modification time changed");
    let modified = std::fs::metadata(&snapshot).unwrap().modified().unwrap();
    {
        use std::io::{Seek, SeekFrom, Write};
        let mut file = File::options().write(true).open(&snapshot).unwrap();
        file.seek(SeekFrom::End(-1)).unwrap();
        file.write_all(&[0x5a]).unwrap();
        file.set_modified(modified).unwrap();
    }
    assert!(attempt(), "bytes changed");
    assert!(!attempt(), "and kept once taken again");
    assert_eq!(snapshots(dir.path()), kept(SCHEMA_VERSION));
}

/// A commit by another connection between the snapshot and the migration
/// transaction refuses the upgrade (the snapshot would not hold it); the
/// next start takes a snapshot that does.
#[test]
fn a_commit_after_the_snapshot_refuses_the_upgrade() {
    fn commit(path: &Path) {
        Connection::open(path)
            .unwrap()
            .execute(
                "INSERT INTO budgets(id,limit_units) VALUES ('between',1)",
                [],
            )
            .unwrap();
    }
    let from = SCHEMA_VERSION - 1;
    let dir = tempfile::tempdir().unwrap();
    let path = store_at(dir.path(), from);
    crate::BEFORE_MIGRATING.set(Some(commit));
    let opened = Store::open(&path);
    crate::BEFORE_MIGRATING.set(None);
    assert_eq!(opened.err().unwrap().0, ErrorCode::StateNotDurable);
    assert_eq!(user_version(&path), from);
    let snapshot = snapshot_path(&path, from).unwrap();
    assert_eq!(budget_rows(&snapshot, "between"), 0);
    events();
    drop(Store::open(&path).unwrap());
    assert_eq!(writes(&events()), 1);
    assert_eq!(budget_rows(&snapshot, "between"), 1);
}

/// One snapshot is kept, with its record. Older ones, records and leftovers
/// of any version go; other files are left alone.
#[test]
fn only_the_newest_snapshot_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let from = SCHEMA_VERSION - 1;
    let path = store_at(dir.path(), from);
    for name in [
        "core.sqlite.v12.snapshot",
        "core.sqlite.v12.snapshot.state",
        "core.sqlite.v15.snapshot.partial",
        "core.sqlite.v15.snapshot.partial-journal",
        "core.sqlite.v15.snapshot.state.partial",
        format!("core.sqlite.v{from}.snapshot.partial").as_str(),
        "core.sqlite.v12.snapshot.keep",
        "core.sqlite.vX.snapshot",
        "other.sqlite.v3.snapshot",
    ] {
        std::fs::write(dir.path().join(name), b"old").unwrap();
    }
    drop(Store::open(&path).unwrap());
    assert_eq!(snapshots(dir.path()), kept(from));
    for kept in [
        "core.sqlite.v12.snapshot.keep",
        "core.sqlite.vX.snapshot",
        "other.sqlite.v3.snapshot",
    ] {
        assert!(dir.path().join(kept).exists(), "{kept}");
    }
    // A store already current takes none and removes none.
    drop(Store::open(&path).unwrap());
    assert_eq!(snapshots(dir.path()), kept(from));
}

#[test]
fn snapshot_file_names_are_classified() {
    for (name, expected) in [
        ("core.sqlite.v16.snapshot", Some((16, Kind::Snapshot))),
        ("core.sqlite.v16.snapshot.state", Some((16, Kind::State))),
        (
            "core.sqlite.v16.snapshot.partial",
            Some((16, Kind::Leftover)),
        ),
        (
            "core.sqlite.v16.snapshot.partial-journal",
            Some((16, Kind::Leftover)),
        ),
        (
            "core.sqlite.v16.snapshot.state.partial",
            Some((16, Kind::Leftover)),
        ),
        ("core.sqlite.v.snapshot", None),
        ("core.sqlite.v16.snapshot.keep", None),
        ("core.sqlite.v99999999999.snapshot", None),
        ("other.sqlite.v16.snapshot", None),
    ] {
        assert_eq!(classify("core.sqlite", name), expected, "{name}");
    }
}

/// `journal_mode=WAL` answers with the mode it kept instead of failing: an
/// in-memory database keeps `memory`, a VFS without shared memory keeps
/// `delete`. `Store::open` itself refuses such a store, writing nothing.
#[test]
fn a_store_that_cannot_use_wal_is_refused() {
    let memory = Connection::open_in_memory().unwrap();
    assert_eq!(
        require_wal(&memory).err().unwrap().0,
        ErrorCode::StateNotDurable
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    let no_shm =
        Connection::open_with_flags_and_vfs(&path, rusqlite::OpenFlags::default(), "unix-none")
            .unwrap();
    let answer: String = no_shm
        .pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))
        .unwrap();
    assert_eq!(answer, "delete", "a silent refusal, not an error");
    drop(no_shm);
    crate::OPEN_VFS.set(Some("unix-none"));
    let opened = Store::open(&path);
    crate::OPEN_VFS.set(None);
    assert_eq!(opened.err().unwrap().0, ErrorCode::StateNotDurable);
    assert_eq!(user_version(&path), 0, "nothing created");
    drop(Store::open(&path).unwrap());
}

/// A version-18 store of `rows` settled container jobs whose intents carry
/// `pad` bytes, packed by `VACUUM`.
fn settled_jobs_store(dir: &Path, rows: u32, pad: u32) -> PathBuf {
    let path = dir.join("core.sqlite");
    let mut store = Store::open(&path).unwrap();
    store.issue_bootstrap("token", 100).unwrap();
    store
        .enroll("token", "alice", "$argon2id$fixture", 101)
        .unwrap();
    let principal = store.login_record("alice").unwrap().unwrap().principal;
    store
        .conn
        .execute(
            "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c WHERE x<?2)
             INSERT INTO jobs(id,principal,idempotency,intent,digest,resource,state,generation,revision,deadline)
             SELECT printf('%064d',x),?1,printf('key-%06d',x),
                 json_object('operation','container_restart','plan',substr(hex(zeroblob(?3)),1,?3)),
                 printf('%064d',x),printf('container:%06d',x),'queued',1,1,4000000000 FROM c",
            params![principal.id, rows, pad],
        )
        .unwrap();
    // Queued first: a job's resource set is written while it is queued.
    store
        .conn
        .execute("UPDATE jobs SET state='succeeded'", [])
        .unwrap();
    crate::remove_v19_schema(&store.conn);
    store
        .conn
        .execute_batch("PRAGMA user_version=18; VACUUM;")
        .unwrap();
    drop(store);
    path
}
/// The leaves of `jobs`: how many, how many hold two records, and the free
/// bytes most two-record leaves have.
fn job_leaves(path: &Path) -> (i64, i64, i64) {
    let conn =
        Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    conn.query_row(
        "SELECT (SELECT count(*) FROM dbstat WHERE name='jobs' AND pagetype='leaf'),
                (SELECT count(*) FROM dbstat WHERE name='jobs' AND pagetype='leaf' AND ncell=2),
                coalesce((SELECT unused FROM dbstat WHERE name='jobs' AND pagetype='leaf' AND ncell=2
                    GROUP BY unused ORDER BY count(*) DESC LIMIT 1),-1)",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .unwrap()
}

/// The schema-19 estimate is an upper bound at its worst case: every leaf
/// of `jobs` holds two records with fewer free bytes than setting
/// `settled` adds, so every leaf splits (one WAL frame for it and one for
/// its sibling, and the sibling added to the database file). Admitted with
/// exactly the space it asks for, the upgrade leaves the write reserve free
/// at its peak (after the commit's checkpoint, before the WAL is reset:
/// database, WAL, snapshot and record as they stand when open returns);
/// a bound from rows alone would not have covered it; and the estimate asks
/// for no more than twice what it uses.
#[test]
fn the_v19_estimate_covers_every_job_leaf_splitting() {
    // Find the padding that leaves 4 or 5 free bytes on a two-record leaf:
    // fewer than the 10 that two records gain (a serial type byte and a
    // 4-byte integer each).
    let probe = tempfile::tempdir().unwrap();
    let (_, two, free) = job_leaves(&settled_jobs_store(probe.path(), 40, 1650));
    assert!(
        two > 0 && free >= 10,
        "calibration: {two} leaves, {free} free"
    );
    let pad = 1650 + u32::try_from(free / 2 - 2).unwrap();
    drop(probe);
    let dir = tempfile::tempdir().unwrap();
    let path = settled_jobs_store(dir.path(), 3000, pad);
    let (leaves, two, free) = job_leaves(&path);
    assert!(
        two * 100 >= leaves * 95,
        "{two} of {leaves} leaves hold two"
    );
    assert!((0..10).contains(&free), "{free} bytes free");
    let row_bound: i64 = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT (sum(octet_length(id)+octet_length(principal)+octet_length(idempotency)+octet_length(intent)+octet_length(digest)+octet_length(resource)+octet_length(state))+count(*)*104)*8/3 FROM jobs",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let on_disk = || -> u64 {
        [
            path.clone(),
            path.with_extension("sqlite-wal"),
            snapshot_path(&path, 18).unwrap(),
            state_path(&path, 18).unwrap(),
        ]
        .iter()
        .map(|file| std::fs::metadata(file).map_or(0, |m| m.len()))
        .sum()
    };
    crate::OPEN_FREE_BYTES.set(Some(0));
    let needed = crate::upgrade_space(&path).unwrap().needed;
    crate::OPEN_FREE_BYTES.set(Some(needed));
    let before = on_disk();
    let opened = Store::open(&path);
    crate::OPEN_FREE_BYTES.set(None);
    let store = opened.unwrap();
    // Measured with the store still open: the WAL keeps its size until the
    // next write (closing would checkpoint and remove it).
    let used = on_disk() - before;
    drop(store);
    let snapshot = std::fs::metadata(snapshot_path(&path, 18).unwrap())
        .unwrap()
        .len();
    let (after, _, _) = job_leaves(&path);
    assert!(after * 10 >= leaves * 19, "{leaves} leaves became {after}");
    assert!(
        used + MIN_FREE_BYTES <= needed,
        "used {used}, allowed {}",
        needed - MIN_FREE_BYTES
    );
    assert!(
        used - snapshot > u64::try_from(row_bound).unwrap(),
        "a bound from rows ({row_bound}) would have covered {}",
        used - snapshot
    );
    assert!(
        needed - MIN_FREE_BYTES <= 2 * used,
        "used {used}, allowed {}",
        needed - MIN_FREE_BYTES
    );
}
