//! The receipt store's schema, held whole to what its migrations build.
//!
//! `quick_check` checks structure, not definitions: a store missing the
//! partial UNIQUE index `actions_resource_lock`, or holding it without
//! `UNIQUE` or with another `WHERE`, passes it, and its column `SELECT`s
//! still prepare, yet every insert relies on that index to keep a resource
//! exclusive to one unresolved action. The same holds for
//! `stack_actions_lock`, the primary keys' automatic indexes (action ids
//! unique), the `CHECK`s in each table's text (states, operations,
//! closures, the effect log's kinds and outcomes), the digest and effect-log
//! indexes retention and the budget read through, and any trigger, which
//! could rewrite a row behind every check.
//!
//! So `open` compares every row of `sqlite_schema` (type, name, table and
//! SQL text) with a reference built in memory by running `migrate` from an
//! empty database, exactly as every real store was built: an empty store is
//! created at version 1 and migrated, so a store of any version has run the
//! same statements, and SQLite stores `CREATE` text as written and edits it
//! for `ADD COLUMN` the same way in every bundled version. Rows are matched
//! by type and name in both directions (a trigger may share a table's or an
//! index's name: SQLite keeps triggers in their own namespace), so every row
//! of the store is accounted for and nothing hides behind a name. Any
//! missing, altered, duplicated or extra object refuses the open, naming
//! the object. The only rows allowed beyond the reference are the query
//! planner's statistics tables an `ANALYZE` creates (`sqlite_stat1`, and
//! `sqlite_stat4` in this build), exactly as this SQLite defines them: they
//! steer plans, never results. `sqlite_sequence` is not allowed: no table
//! here uses `AUTOINCREMENT`, so it appears only after a foreign table did.
//! The reference is built once per process; the store's schema pages are
//! already in the page cache from `quick_check`.
//!
//! Migrations run before that comparison, so before any migration runs
//! `before_migrating` refuses a trigger or view: the migrations are fixed
//! statements, and a trigger is the only schema object through which they
//! could change rows they do not name (a missing, extra or altered table or
//! index makes a step fail and roll back, or is refused after it). A store
//! refused there keeps its rows and version. It is not left byte-identical:
//! `open` has already set the file's mode and WAL mode, as it does for any
//! refusal, including one at the current version.
use crate::{Connection, Error, ErrorCode, Result, migrate};
use std::sync::OnceLock;

/// One `sqlite_schema` row: type, name, table, SQL (absent for automatic
/// indexes).
type Row = (String, String, String, Option<String>);

fn rows(conn: &Connection) -> rusqlite::Result<Vec<Row>> {
    let mut statement =
        conn.prepare("SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY type,name")?;
    statement
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect()
}

/// The schema every store at `RECEIPT_SCHEMA_VERSION` holds, and the
/// statistics tables an `ANALYZE` may add to it.
struct Reference {
    required: Vec<Row>,
    statistics: Vec<Row>,
}
fn build() -> Result<Reference> {
    let unavailable = |_| Error(ErrorCode::Unavailable);
    let conn = Connection::open_in_memory().map_err(unavailable)?;
    // Short-lived and holding only the schema; a small cache all the same.
    conn.execute_batch("PRAGMA cache_size=-64;")
        .map_err(unavailable)?;
    migrate(&conn, 0)?;
    let required = rows(&conn).map_err(unavailable)?;
    conn.execute_batch("ANALYZE;").map_err(unavailable)?;
    let statistics = rows(&conn)
        .map_err(unavailable)?
        .into_iter()
        .filter(|row| !required.contains(row))
        .collect();
    Ok(Reference {
        required,
        statistics,
    })
}
fn reference() -> Result<&'static Reference> {
    static REFERENCE: OnceLock<Reference> = OnceLock::new();
    if let Some(reference) = REFERENCE.get() {
        return Ok(reference);
    }
    let built = build()?;
    Ok(REFERENCE.get_or_init(|| built))
}

/// The first difference between a store's schema and the reference, naming
/// the object; `None` when every row of each is accounted for.
fn difference(actual: &[Row], reference: &Reference) -> Option<String> {
    let version = crate::RECEIPT_SCHEMA_VERSION;
    for (kind, name, table, sql) in &reference.required {
        match actual.iter().find(|row| &row.0 == kind && &row.1 == name) {
            None => return Some(format!("{kind} {name} is missing")),
            Some((_, _, t, s)) if t != table || s != sql => {
                return Some(format!(
                    "{kind} {name} differs from the definition schema {version} expects"
                ));
            }
            Some(_) => {}
        }
    }
    let mut seen: Vec<(&str, &str)> = Vec::with_capacity(actual.len());
    for row in actual {
        let (kind, name, ..) = row;
        let key = (kind.as_str(), name.as_str());
        if seen.contains(&key) {
            return Some(format!("{kind} {name} appears twice"));
        }
        seen.push(key);
        let required = reference
            .required
            .iter()
            .any(|e| &e.0 == kind && &e.1 == name);
        if !required {
            match reference
                .statistics
                .iter()
                .find(|e| &e.0 == kind && &e.1 == name)
            {
                Some(expected) if expected == row => {}
                Some(_) => {
                    return Some(format!(
                        "{kind} {name} differs from SQLite's own definition"
                    ));
                }
                None => return Some(format!("unexpected {kind} {name}")),
            }
        }
    }
    None
}

fn refuse(detail: String) -> Refusal {
    Refusal {
        error: Error(ErrorCode::StateNotDurable),
        schema: Some(SchemaMismatch(detail)),
    }
}

/// Before any migration: no trigger or view, which no version of the
/// schema has.
pub(crate) fn before_migrating(conn: &Connection) -> std::result::Result<(), Refusal> {
    let actual = rows(conn).map_err(|_| Error(ErrorCode::StateNotDurable))?;
    match actual
        .iter()
        .find(|(kind, ..)| kind == "trigger" || kind == "view")
    {
        Some((kind, name, ..)) => Err(refuse(format!("unexpected {kind} {name}"))),
        None => Ok(()),
    }
}

/// Why a store's schema is refused, for the operator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaMismatch(pub String);
impl std::fmt::Display for SchemaMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "receipt store schema: {}", self.0)
    }
}

/// Hold `conn`'s schema to the reference.
pub(crate) fn check(conn: &Connection) -> std::result::Result<(), Refusal> {
    let reference = reference()?;
    let actual = rows(conn).map_err(|_| Error(ErrorCode::StateNotDurable))?;
    match difference(&actual, reference) {
        None => Ok(()),
        Some(detail) => Err(refuse(detail)),
    }
}

/// Why `ReceiptStore::open` refused: the error it answers with, and the
/// schema object at fault when that is the reason.
#[derive(Debug)]
pub struct Refusal {
    pub error: Error,
    pub schema: Option<SchemaMismatch>,
}
impl From<Error> for Refusal {
    fn from(error: Error) -> Self {
        Self {
            error,
            schema: None,
        }
    }
}

#[cfg(test)]
mod tests;
