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
//! for `ADD COLUMN` the same way in every bundled version. Any missing,
//! altered or extra object refuses the open, naming the object. The
//! reference costs a few in-memory statements; the store's schema pages are
//! already in the page cache from `quick_check`.
use crate::{Connection, Error, ErrorCode, Result, migrate};

/// One `sqlite_schema` row: type, name, table, SQL (absent for automatic
/// indexes).
type Row = (String, String, String, Option<String>);

fn rows(conn: &Connection) -> rusqlite::Result<Vec<Row>> {
    let mut statement =
        conn.prepare("SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY name")?;
    statement
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect()
}

/// The schema every store at `RECEIPT_SCHEMA_VERSION` holds.
fn reference() -> Result<Vec<Row>> {
    let conn = Connection::open_in_memory().map_err(|_| Error(ErrorCode::Unavailable))?;
    // Short-lived and holding only the schema; a small cache all the same.
    conn.execute_batch("PRAGMA cache_size=-64;")
        .map_err(|_| Error(ErrorCode::Unavailable))?;
    migrate(&conn, 0)?;
    rows(&conn).map_err(|_| Error(ErrorCode::Unavailable))
}

/// The first difference between a store's schema and the reference, naming
/// the object; `None` when they are equal.
fn difference(actual: &[Row], expected: &[Row]) -> Option<String> {
    for (kind, name, table, sql) in expected {
        match actual.iter().find(|row| &row.1 == name) {
            None => return Some(format!("{kind} {name} is missing")),
            Some((k, _, t, s)) if k != kind || t != table || s != sql => {
                return Some(format!(
                    "{kind} {name} differs from the definition schema {} expects",
                    crate::RECEIPT_SCHEMA_VERSION
                ));
            }
            Some(_) => {}
        }
    }
    actual
        .iter()
        .find(|row| !expected.iter().any(|e| e.1 == row.1))
        .map(|(kind, name, ..)| format!("unexpected {kind} {name}"))
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
    let expected = reference()?;
    let actual = rows(conn).map_err(|_| Error(ErrorCode::StateNotDurable))?;
    match difference(&actual, &expected) {
        None => Ok(()),
        Some(detail) => Err(Refusal {
            error: Error(ErrorCode::StateNotDurable),
            schema: Some(SchemaMismatch(detail)),
        }),
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
