use super::*;
use std::os::unix::fs::PermissionsExt;

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
/// The store's snapshot files in `dir`, finished or partial, by name.
fn snapshots(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|name| snapshot_version("core.sqlite", name).is_some())
        .collect();
    names.sort();
    names
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
    for migration in MIGRATIONS {
        for object in migration.checked {
            assert!(
                stored_sql(&reference, object.kind(), object.name())
                    .unwrap()
                    .is_some(),
                "{object:?}"
            );
        }
        for table in migration.rewrites {
            assert!(stored_sql(&reference, "table", table).unwrap().is_some());
        }
    }
}

/// The data path is unchanged by the table: a store at any older version
/// migrates to exactly the schema a new store gets, and each upgrade leaves
/// one snapshot, at the version it started from, private to the store's
/// owner and openable.
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
        let snapshot = snapshot_path(&path, from).unwrap();
        assert_eq!(
            snapshots(dir.path()),
            [format!("core.sqlite.v{from}.snapshot")]
        );
        let mode = std::fs::metadata(&snapshot).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "from version {from}");
        assert_eq!(user_version(&snapshot), from);
    }
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
    assert_eq!(
        snapshots(dir.path()),
        [format!("core.sqlite.v{SCHEMA_VERSION}.snapshot")]
    );
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
    tamper(
        "PRAGMA writable_schema=ON; UPDATE sqlite_schema SET sql=replace(sql,'<=9','<=8') WHERE name='probe'; PRAGMA writable_schema=RESET;",
    );
    Store::open_migrating(&path, &extended).unwrap();
    tamper("DROP TRIGGER probe_kept");
    assert_eq!(
        Store::open_migrating(&path, &extended).err().unwrap().0,
        ErrorCode::StateNotDurable
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
    let mut extended = MIGRATIONS.to_vec();
    extended.push(Migration {
        version: SCHEMA_VERSION + 1,
        description: "test only: forgets its version",
        sql: "CREATE TABLE probe (id INTEGER PRIMARY KEY) STRICT;",
        checked: &[],
        rewrites: &[],
    });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    drop(Store::open(&path).unwrap());
    assert_eq!(
        Store::open_migrating(&path, &extended).err().unwrap().0,
        ErrorCode::StateNotDurable
    );
    assert_eq!(user_version(&path), SCHEMA_VERSION);
}

/// Version 16 widens the `jobs` state CHECK by editing its text; when the
/// text is not what it expects, its `replace` changes nothing and SQLite
/// reports no error. Without a check the store reached version 19 with no
/// `resolved` state, so no stuck job could ever be resolved.
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
    assert_eq!(user_version(&path), 15);
    // Version 16's SQL alone runs without error and leaves the CHECK as it
    // was: what the check after it is for.
    let mut conn = Connection::open(&path).unwrap();
    let tx = conn.transaction().unwrap();
    tx.execute_batch(MIGRATIONS[15].sql).unwrap();
    let jobs = stored_sql(&tx, "table", "jobs").unwrap().unwrap();
    assert!(!jobs.contains("'resolved'"), "{jobs}");
    assert_eq!(
        verify(&tx, MIGRATIONS, 15).err().unwrap().0,
        ErrorCode::StateNotDurable
    );
}

/// A current store whose `jobs` table lost the `resolved` state (or
/// changed in any other way) is refused at startup.
#[test]
fn a_changed_jobs_table_refuses_startup() {
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
    let rows = |file: &Path| -> i64 {
        Connection::open(file)
            .unwrap()
            .query_row("SELECT count(*) FROM budgets WHERE id='in-wal'", [], |r| {
                r.get(0)
            })
            .unwrap()
    };
    assert_eq!(rows(&bare), 0, "the database file alone misses it");
    drop(Store::open(&path).unwrap());
    let snapshot = snapshot_path(&path, SCHEMA_VERSION - 1).unwrap();
    // Restored as the operator would: copied over the store's name in an
    // empty directory, where the current build opens and migrates it.
    let restored = tempfile::tempdir().unwrap();
    let store = restored.path().join("core.sqlite");
    std::fs::copy(&snapshot, &store).unwrap();
    assert_eq!(rows(&store), 1);
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
    assert!(
        !dir.path()
            .join(format!("core.sqlite.v{from}.snapshot.partial"))
            .exists()
    );
    std::fs::remove_dir_all(&blocked).unwrap();
    drop(Store::open(&path).unwrap());
    assert_eq!(user_version(&path), SCHEMA_VERSION);
    assert!(blocked.is_file());
}

/// Short of space for the snapshot and the upgrade, the open is refused
/// before writing anything, and `upgrade_space` says how much it needs.
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
    drop(conn);
    crate::OPEN_FREE_BYTES.set(Some(0));
    let needed = crate::upgrade_space(&path).unwrap().needed;
    assert_eq!(needed, bytes + MIN_FREE_BYTES);
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
    let snapshot = snapshot_path(&path, from).unwrap();
    let written = std::fs::metadata(&snapshot).unwrap().len();
    assert!(written <= bytes, "estimated {bytes}, wrote {written}");
    assert_eq!(crate::upgrade_space(&path), None);
}

/// One snapshot is kept: the newest. Older ones and partial files of any
/// version go once it is written; other files are left alone.
#[test]
fn only_the_newest_snapshot_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let from = SCHEMA_VERSION - 1;
    let path = store_at(dir.path(), from);
    for name in [
        "core.sqlite.v12.snapshot",
        "core.sqlite.v15.snapshot.partial",
        format!("core.sqlite.v{from}.snapshot.partial").as_str(),
        "core.sqlite.v12.snapshot.keep",
        "core.sqlite.vX.snapshot",
        "other.sqlite.v3.snapshot",
    ] {
        std::fs::write(dir.path().join(name), b"old").unwrap();
    }
    drop(Store::open(&path).unwrap());
    assert_eq!(
        snapshots(dir.path()),
        [format!("core.sqlite.v{from}.snapshot")]
    );
    for kept in [
        "core.sqlite.v12.snapshot.keep",
        "core.sqlite.vX.snapshot",
        "other.sqlite.v3.snapshot",
    ] {
        assert!(dir.path().join(kept).exists(), "{kept}");
    }
    // A store already current takes none and removes none.
    drop(Store::open(&path).unwrap());
    assert_eq!(
        snapshots(dir.path()),
        [format!("core.sqlite.v{from}.snapshot")]
    );
}

/// `journal_mode=WAL` answers with the mode it kept instead of failing: an
/// in-memory database and a VFS without shared memory keep theirs.
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
    assert_eq!(
        require_wal(&no_shm).err().unwrap().0,
        ErrorCode::StateNotDurable
    );
    drop(no_shm);
    require_wal(&Connection::open(&path).unwrap()).unwrap();
}
