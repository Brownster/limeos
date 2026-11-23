//! The store's schema, as one ordered table of migrations.
//!
//! `Store::open` applies the entries above the store's `user_version`, in
//! order, and the startup schema check (`resources::check`) builds its
//! reference schema from the same entries and compares the objects they
//! name. A new migration is one more entry here and nothing else.
//!
//! Before any entry is applied to an existing store, `Store::open` writes a
//! snapshot of it beside the store ([`snapshot`]), which an operator can
//! restore (`docs/p03-operations.md`).
use super::*;

/// A schema object whose stored SQL the startup check compares with the
/// reference schema built from the migration table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Object {
    Table(&'static str),
    Index(&'static str),
}
impl Object {
    fn kind(self) -> &'static str {
        match self {
            Self::Table(_) => "table",
            Self::Index(_) => "index",
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Table(name) | Self::Index(name) => name,
        }
    }
}

/// One schema migration.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Migration {
    /// The `user_version` the store has once this has run; its SQL sets it.
    pub(crate) version: u32,
    /// What it adds, for a reader of the table (the tests require one).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) description: &'static str,
    pub(crate) sql: &'static str,
    /// The tables and indexes this migration creates or changes whose SQL
    /// the startup check compares with the reference, on every start.
    /// Every trigger is compared whatever the entries say.
    pub(crate) checked: &'static [Object],
    /// Tables this migration edits as text through `writable_schema`, which
    /// SQLite neither parses nor checks against what was meant: compared
    /// with the reference at this version as soon as it has run, before
    /// any later migration builds on them.
    pub(crate) rewrites: &'static [&'static str],
}

const fn plain(version: u32, description: &'static str, sql: &'static str) -> Migration {
    Migration {
        version,
        description,
        sql,
        checked: &[],
        rewrites: &[],
    }
}
const fn checked(
    version: u32,
    description: &'static str,
    sql: &'static str,
    checked: &'static [Object],
) -> Migration {
    Migration {
        version,
        description,
        sql,
        checked,
        rewrites: &[],
    }
}

use Object::{Index, Table};
/// Every migration, in order: version `n` is entry `n - 1`.
pub(crate) const MIGRATIONS: &[Migration] = &[
    plain(
        1,
        "authority schema: users, grants, sessions, jobs, events, task tokens, budgets",
        include_str!("schema.sql"),
    ),
    plain(2, "restart plans", include_str!("migration-v2.sql")),
    plain(3, "restart results", include_str!("migration-v3.sql")),
    plain(
        4,
        "restart plans and results become container plans and results",
        include_str!("migration-v4.sql"),
    ),
    plain(5, "compose plans", include_str!("migration-v5.sql")),
    plain(6, "storage preview plans", include_str!("migration-v6.sql")),
    plain(
        7,
        "job resource sets and resource locks",
        include_str!("migration-v7.sql"),
    ),
    checked(
        8,
        "storage target preparation plans and results",
        include_str!("migration-v8.sql"),
        &[
            Table("storage_target_plans"),
            Table("storage_target_results"),
        ],
    ),
    checked(
        9,
        "backup preparation plans",
        include_str!("migration-v9.sql"),
        &[Table("backup_preparation_plans")],
    ),
    checked(
        10,
        "storage mount plans and results",
        include_str!("migration-v10.sql"),
        &[Table("storage_mount_plans"), Table("storage_mount_results")],
    ),
    checked(
        11,
        "automation schedules and occurrences",
        include_str!("migration-v11.sql"),
        &[
            Table("automation_schedules"),
            Table("automation_occurrences"),
        ],
    ),
    checked(
        12,
        "automation observation settlement: streams, results, alerts, notifications",
        include_str!("migration-v12.sql"),
        &[
            Table("automation_streams"),
            Table("automation_observation_results"),
            Table("automation_alerts"),
            Table("automation_notifications"),
        ],
    ),
    checked(
        13,
        "configured health policies",
        include_str!("migration-v13.sql"),
        &[Table("automation_health_policies")],
    ),
    checked(
        14,
        "automation poll origins",
        include_str!("migration-v14.sql"),
        &[Table("automation_poll_origins")],
    ),
    checked(
        15,
        "stack plans and results",
        include_str!("migration-v15.sql"),
        &[Table("stack_plans"), Table("stack_results")],
    ),
    // The only migration that edits a table's text in place: it widens the
    // `jobs` state CHECK with a `replace` that silently does nothing if the
    // text is not exactly what it expects (or already names 'resolved').
    // Version 19's triggers and job resolution assume the widened CHECK.
    Migration {
        version: 16,
        description: "job resolution: the resolved job state and job_resolutions",
        sql: include_str!("migration-v16.sql"),
        checked: &[Table("job_resolutions"), Table("jobs")],
        rewrites: &["jobs"],
    },
    checked(
        17,
        "event indexes by job, kind and notification",
        include_str!("migration-v17.sql"),
        &[
            Index("events_job"),
            Index("events_kind"),
            Index("events_notification"),
        ],
    ),
    plain(
        18,
        "the container executor's receipt-store epoch",
        include_str!("migration-v18.sql"),
    ),
    checked(
        19,
        "job-row retention: settlement time, delete guards, retention index",
        include_str!("migration-v19.sql"),
        &[Index("jobs_retention")],
    ),
];

/// The schema `migrations` build from nothing, in a private in-memory
/// database.
pub(crate) fn reference(migrations: &[Migration]) -> Result<Connection> {
    let reference = Connection::open_in_memory().map_err(durable)?;
    for migration in migrations {
        reference.execute_batch(migration.sql).map_err(durable)?;
    }
    Ok(reference)
}

fn stored_sql(conn: &Connection, kind: &str, name: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT sql FROM sqlite_schema WHERE type=? AND name=?",
        [kind, name],
        |r| r.get(0),
    )
    .optional()
    .map_err(durable)
}

/// Whether `conn` has every trigger of the reference, and every table and
/// index the migrations name, with exactly the reference's SQL. A version
/// number and healthy pages do not prove that dispatch and release guards
/// survived.
pub(crate) fn check_schema(conn: &Connection, migrations: &[Migration]) -> Result<()> {
    let reference = reference(migrations)?;
    let mut stmt = reference
        .prepare("SELECT name,sql FROM sqlite_schema WHERE type='trigger'")
        .map_err(durable)?;
    let triggers = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(durable)?;
    for trigger in triggers {
        let (name, expected) = trigger.map_err(durable)?;
        if stored_sql(conn, "trigger", &name)?.as_ref() != Some(&expected) {
            return Err(Error(ErrorCode::StateNotDurable));
        }
    }
    for object in migrations.iter().flat_map(|m| m.checked) {
        let expected = stored_sql(&reference, object.kind(), object.name())?
            .ok_or(Error(ErrorCode::StateNotDurable))?;
        if stored_sql(conn, object.kind(), object.name())?.as_ref() != Some(&expected) {
            return Err(Error(ErrorCode::StateNotDurable));
        }
    }
    Ok(())
}

/// Applies `migrations[index]` inside the startup transaction and confirms
/// what SQLite will not: that it set its version, and that every table it
/// rewrote as text is now exactly the reference at that version.
pub(crate) fn apply(tx: &Transaction<'_>, migrations: &[Migration], index: usize) -> Result<()> {
    let migration = &migrations[index];
    tx.execute_batch(migration.sql).map_err(durable)?;
    verify(tx, migrations, index)
}
/// The checks [`apply`] makes after running `migrations[index]`.
pub(crate) fn verify(tx: &Transaction<'_>, migrations: &[Migration], index: usize) -> Result<()> {
    let migration = &migrations[index];
    let version: u32 = tx
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(durable)?;
    if version != migration.version {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    if migration.rewrites.is_empty() {
        return Ok(());
    }
    let reference = reference(&migrations[..=index])?;
    for table in migration.rewrites {
        let expected =
            stored_sql(&reference, "table", table)?.ok_or(Error(ErrorCode::StateNotDurable))?;
        if stored_sql(tx, "table", table)?.as_ref() != Some(&expected) {
            return Err(Error(ErrorCode::StateNotDurable));
        }
    }
    Ok(())
}

/// The file a snapshot of the store at `path`, taken at schema `from`, is
/// written to: `core.sqlite.v16.snapshot` beside `core.sqlite`.
pub(crate) fn snapshot_path(path: &Path, from: u32) -> Option<PathBuf> {
    let name = path.file_name()?.to_str()?;
    Some(path.with_file_name(format!("{name}.v{from}.snapshot")))
}
/// The schema version of a snapshot of the store named `store`, finished or
/// partial, if `file` is one.
fn snapshot_version(store: &str, file: &str) -> Option<u32> {
    let rest = file.strip_prefix(store)?.strip_prefix(".v")?;
    let version = rest
        .strip_suffix(".snapshot")
        .or_else(|| rest.strip_suffix(".snapshot.partial"))?;
    if version.is_empty() || !version.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    version.parse().ok()
}

/// Bytes a snapshot of the store will take: its pages in use (`VACUUM INTO`
/// leaves out the free ones).
pub(crate) fn snapshot_bytes(conn: &Connection) -> Result<u64> {
    let (pages, free, size): (i64, i64, i64) = conn
        .query_row(
            "SELECT (SELECT page_count FROM pragma_page_count),(SELECT freelist_count FROM pragma_freelist_count),(SELECT page_size FROM pragma_page_size)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(durable)?;
    Ok(u64::try_from(pages.saturating_sub(free).saturating_mul(size)).unwrap_or(0))
}

/// Writes a snapshot of the store at `path` (schema `from`, open on `conn`
/// outside any transaction) to [`snapshot_path`], then removes the store's
/// other snapshots, finished or partial: one is kept, the one taken before
/// the latest upgrade, so they cannot pile up on a small card. The older
/// ones go only once the new one is durable.
///
/// `VACUUM INTO` reads one consistent state of the database through
/// SQLite, the WAL's committed frames included, which a copy of the
/// database file alone would miss. It goes to a partial file first, created
/// here with the store's mode (0600, owned by this process like the store);
/// SQLite does not sync it, so it is synced here, checked (its version and
/// `quick_check`), renamed into place and the directory synced. Any failure
/// removes the partial file and fails, so the upgrade does not start.
pub(crate) fn snapshot(conn: &Connection, path: &Path, from: u32) -> Result<PathBuf> {
    let failed = |_| Error(ErrorCode::StateNotDurable);
    let directory = path.parent().ok_or(Error(ErrorCode::InvalidInput))?;
    let store = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or(Error(ErrorCode::InvalidInput))?;
    let target = snapshot_path(path, from).ok_or(Error(ErrorCode::InvalidInput))?;
    let partial = target.with_file_name(format!("{store}.v{from}.snapshot.partial"));
    match std::fs::remove_file(&partial) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(failed(error)),
    }
    let written = (|| {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
            .open(&partial)
            .map_err(failed)?;
        let into = partial.to_str().ok_or(Error(ErrorCode::InvalidInput))?;
        conn.execute("VACUUM INTO ?", [into]).map_err(durable)?;
        let copy = Connection::open_with_flags(
            &partial,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(durable)?;
        let (version, check): (u32, String) = copy
            .query_row(
                "SELECT (SELECT user_version FROM pragma_user_version),(SELECT quick_check FROM pragma_quick_check)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(durable)?;
        drop(copy);
        if version != from || check != "ok" {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        File::open(&partial)
            .and_then(|file| file.sync_all())
            .map_err(failed)?;
        std::fs::rename(&partial, &target).map_err(failed)?;
        File::open(directory)
            .and_then(|dir| dir.sync_all())
            .map_err(failed)
    })();
    if let Err(error) = written {
        let _ = std::fs::remove_file(&partial);
        return Err(error);
    }
    // The new snapshot is durable: every other one goes, finished or
    // partial. A failure to remove one leaves it for the next upgrade to try
    // again; it never stops this one.
    if let Ok(entries) = std::fs::read_dir(directory) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if snapshot_version(store, name).is_some() && entry.path() != target {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    Ok(target)
}

#[cfg(test)]
mod tests;
