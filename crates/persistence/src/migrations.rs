//! The store's schema, as one ordered table of migrations.
//!
//! `Store::open` applies the entries above the store's `user_version`, in
//! order, and the startup schema check (`resources::check`) builds its
//! reference schema from the same entries and compares the objects they
//! name. A new migration is one more entry here and nothing else.
//!
//! One step stays outside the table: the version-17 index build's free-space
//! check, page-limit test hook and error mapping are keyed on
//! `INDEXED_SCHEMA` in `Store::open`, as is the version-19 estimate in
//! `upgrade_writes`. A migration with its own space cost needs the same.
//!
//! Before any entry is applied to an existing store, `Store::open` keeps a
//! snapshot of it beside the store ([`snapshot`]), which an operator can
//! restore (`docs/p03-operations.md`).
use super::*;

pub(crate) mod snapshot;

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
    /// with the reference at this version as soon as it has run. The final
    /// check would refuse the same store, but only after every later
    /// migration had run (and spilled its pages to the WAL) inside the
    /// doomed transaction, on every restart; this refuses before them.
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
    // Version 19's triggers and job resolution assume the widened CHECK, so
    // `jobs` is compared right after this step and at the end of every open.
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

/// The first object of `conn` that differs from the reference `migrations`
/// build: a trigger of the reference, or a table or index the migrations
/// name, missing or with other SQL. A version number and healthy pages do
/// not prove that dispatch and release guards survived.
pub(crate) fn mismatch(
    conn: &Connection,
    migrations: &[Migration],
) -> Result<Option<SchemaMismatch>> {
    let reference = reference(migrations)?;
    let mut stmt = reference
        .prepare("SELECT name,sql FROM sqlite_schema WHERE type='trigger' ORDER BY name")
        .map_err(durable)?;
    let triggers = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(durable)?;
    for trigger in triggers {
        let (name, expected) = trigger.map_err(durable)?;
        if stored_sql(conn, "trigger", &name)?.as_ref() != Some(&expected) {
            return Ok(Some(SchemaMismatch {
                kind: "trigger",
                name,
            }));
        }
    }
    for object in migrations.iter().flat_map(|m| m.checked) {
        let expected = stored_sql(&reference, object.kind(), object.name())?
            .ok_or(Error(ErrorCode::StateNotDurable))?;
        if stored_sql(conn, object.kind(), object.name())?.as_ref() != Some(&expected) {
            return Ok(Some(SchemaMismatch {
                kind: object.kind(),
                name: object.name().to_owned(),
            }));
        }
    }
    Ok(None)
}
/// [`mismatch`] as a refusal.
#[cfg(test)]
pub(crate) fn check_schema(conn: &Connection, migrations: &[Migration]) -> Result<()> {
    match mismatch(conn, migrations)? {
        None => Ok(()),
        Some(_) => Err(Error(ErrorCode::StateNotDurable)),
    }
}

/// Applies `migrations[index]` inside the startup transaction and confirms
/// what SQLite will not: that it set its version, and that every table it
/// rewrote as text is now exactly the reference at that version.
pub(crate) fn apply(
    tx: &Transaction<'_>,
    path: &Path,
    migrations: &[Migration],
    index: usize,
) -> Result<()> {
    tx.execute_batch(migrations[index].sql).map_err(durable)?;
    verify(tx, path, migrations, index)
}
/// The checks [`apply`] makes after running `migrations[index]`.
/// A rewritten table that differs is noted for the daemon to name.
pub(crate) fn verify(
    tx: &Transaction<'_>,
    path: &Path,
    migrations: &[Migration],
    index: usize,
) -> Result<()> {
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
            crate::note_mismatch(
                path,
                SchemaMismatch {
                    kind: "table",
                    name: (*table).to_owned(),
                },
            );
            return Err(Error(ErrorCode::StateNotDurable));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
