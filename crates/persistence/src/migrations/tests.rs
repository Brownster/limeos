use super::snapshot::{Kind, classify, snapshot_bytes, snapshot_path, state_path};
use super::*;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::time::UNIX_EPOCH;

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
/// Marks a file with an mtime no write can give it, to tell later whether
/// it was written again.
const MARK: Duration = Duration::from_secs(1_000_000);
fn mark(file: &Path) {
    File::options()
        .write(true)
        .open(file)
        .unwrap()
        .set_modified(UNIX_EPOCH + MARK)
        .unwrap();
}
fn marked(file: &Path) -> bool {
    std::fs::metadata(file).unwrap().modified().unwrap() == UNIX_EPOCH + MARK
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
/// `resolved` state, so no stuck job could ever be resolved. The startup
/// comparison of `jobs` (in version 16's `checked`) refuses it and names
/// the table; remove `Table("jobs")` there and this fails.
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

/// A snapshot that cannot be written stops the upgrade: the store keeps its
/// version and no partial file is left.
#[test]
fn a_failed_snapshot_stops_the_migration() {
    let dir = tempfile::tempdir().unwrap();
    let from = SCHEMA_VERSION - 1;
    let path = store_at(dir.path(), from);
    let blocked = snapshot_path(&path, from).unwrap();
    std::fs::create_dir(&blocked).unwrap();
    std::fs::write(blocked.join("occupied"), b"x").unwrap();
    assert_eq!(
        Store::open(&path).err().unwrap().0,
        ErrorCode::StateNotDurable
    );
    assert_eq!(user_version(&path), from);
    assert_eq!(
        snapshots(dir.path()),
        [format!("core.sqlite.v{from}.snapshot")]
    );
    assert!(blocked.is_dir(), "only the directory in the way remains");
    std::fs::remove_dir_all(&blocked).unwrap();
    drop(Store::open(&path).unwrap());
    assert_eq!(user_version(&path), SCHEMA_VERSION);
    assert_eq!(snapshots(dir.path()), kept(from));
}

/// Short of space for the snapshot and the upgrade, the open is refused
/// before writing anything, and `upgrade_space` reports the figures open
/// used: the new snapshot plus the migrations' writes and the reserve.
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
    let (build, retention) = upgrade_writes(&conn, from).unwrap();
    drop(conn);
    crate::OPEN_FREE_BYTES.set(Some(0));
    let needed = crate::upgrade_space(&path).unwrap().needed;
    assert_eq!(needed, bytes + upgrade_remaining(from, build, retention));
    crate::OPEN_FREE_BYTES.set(Some(needed - 1));
    assert_eq!(
        crate::upgrade_space(&path),
        Some(UpgradeSpace {
            from,
            needed,
            available: needed - 1
        })
    );
    assert_eq!(
        Store::open(&path).err().unwrap().0,
        ErrorCode::StateNotDurable
    );
    assert_eq!(user_version(&path), from);
    assert!(snapshots(dir.path()).is_empty());
    crate::OPEN_FREE_BYTES.set(Some(needed));
    assert_eq!(crate::upgrade_space(&path), None);
    drop(Store::open(&path).unwrap());
    crate::OPEN_FREE_BYTES.set(None);
    let written = std::fs::metadata(snapshot_path(&path, from).unwrap())
        .unwrap()
        .len();
    assert!(written <= bytes, "estimated {bytes}, wrote {written}");
    assert_eq!(crate::upgrade_space(&path), None);
}

/// On a disk that holds the store, an interrupted snapshot's leftovers, a
/// stale snapshot of the same version or an older version's snapshot, and
/// exactly enough besides for the new snapshot and the migrations, the
/// upgrade goes through: those files are counted as free and removed
/// before the new snapshot is written. With less, it is refused.
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
        let store = std::fs::metadata(&path).unwrap().len();
        let leave = |size: u64| {
            std::fs::write(dir.path().join(&stale), vec![7u8; size as usize]).unwrap();
            std::fs::write(
                dir.path()
                    .join(format!("core.sqlite.v{from}.snapshot.partial-journal")),
                b"j",
            )
            .unwrap();
        };
        // The open's own WAL index (32 KiB) and some slack: far less than
        // a second copy (`needed` bytes) would need.
        let capacity = store + needed + 64 * 1024;
        leave(needed);
        crate::OPEN_DISK_CAPACITY.set(Some(capacity - needed / 2));
        assert_eq!(
            Store::open(&path).err().unwrap().0,
            ErrorCode::StateNotDurable,
            "{stale}"
        );
        assert_eq!(user_version(&path), from);
        leave(needed);
        crate::OPEN_DISK_CAPACITY.set(Some(capacity));
        let opened = Store::open(&path);
        crate::OPEN_DISK_CAPACITY.set(None);
        drop(opened.unwrap());
        assert_eq!(user_version(&path), SCHEMA_VERSION, "{stale}");
        assert_eq!(snapshots(dir.path()), kept(from), "{stale}");
    }
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
    assert!(Store::open_migrating(&path, &extended).is_err());
    let (snapshot, state) = (
        snapshot_path(&path, SCHEMA_VERSION).unwrap(),
        state_path(&path, SCHEMA_VERSION).unwrap(),
    );
    mark(&snapshot);
    mark(&state);
    let inode = std::fs::metadata(&snapshot).unwrap().ino();
    for _ in 0..5 {
        assert_eq!(
            Store::open_migrating(&path, &extended).err().unwrap().0,
            ErrorCode::StateNotDurable
        );
    }
    assert!(marked(&snapshot) && marked(&state), "nothing written again");
    assert_eq!(std::fs::metadata(&snapshot).unwrap().ino(), inode);
    assert_eq!(snapshots(dir.path()), kept(SCHEMA_VERSION));
    assert_eq!(user_version(&path), SCHEMA_VERSION);
}

/// The snapshot is taken again whenever it cannot be shown to match the
/// store: after a commit (checkpointed, or only in the WAL), and with its
/// record missing, unreadable, for another version or for other bytes.
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
        mark(&snapshot);
        assert!(Store::open_migrating(&path, &extended).is_err());
        !marked(&snapshot)
    };
    assert!(Store::open_migrating(&path, &extended).is_err());
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
    // The snapshot's bytes.
    let mut bytes = std::fs::read(&snapshot).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(&snapshot, bytes).unwrap();
    assert!(attempt(), "snapshot changed");
    assert!(!attempt(), "and kept once taken again");
    assert_eq!(snapshots(dir.path()), kept(SCHEMA_VERSION));
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

/// The estimate for an upgrade from version 18 follows what it writes: the
/// snapshot, and version 19's settlement times and retention index over
/// every settled job (WAL and database file). Admitted with exactly the
/// space it asks for, it leaves the write reserve free, and it asks for no
/// more than three times what it uses.
#[test]
fn the_v18_upgrade_estimate_covers_what_it_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    let mut store = Store::open(&path).unwrap();
    store.issue_bootstrap("token", 100).unwrap();
    store
        .enroll("token", "alice", "$argon2id$fixture", 101)
        .unwrap();
    let principal = store.login_record("alice").unwrap().unwrap().principal;
    store
        .conn
        .execute(
            "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c WHERE x<20000)
             INSERT INTO jobs(id,principal,idempotency,intent,digest,resource,state,generation,revision,deadline)
             SELECT printf('%064d',x),?,printf('key-%d',x),
                 json_object('operation','container_restart','plan',hex(randomblob(120))),
                 hex(randomblob(32)),printf('container:%d',x),'queued',1,1,4000000000 FROM c",
            [&principal.id],
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
    // The commit's checkpoint has copied the WAL into the database, and
    // the WAL keeps its size until the next write: the peak.
    let used = on_disk() - before;
    let settled: i64 = store
        .conn
        .query_row(
            "SELECT count(*) FROM jobs WHERE settled IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    drop(store);
    assert_eq!(settled, 20000);
    assert!(
        used + MIN_FREE_BYTES <= needed,
        "used {used}, allowed {}",
        needed - MIN_FREE_BYTES
    );
    assert!(
        needed - MIN_FREE_BYTES <= 3 * used,
        "used {used}, allowed {}",
        needed - MIN_FREE_BYTES
    );
}
