//! Startup's accounting of indexes against their tables, for what locks,
//! recovery and dispatch read through an index.
//!
//! `PRAGMA quick_check` (SQLite 3.54.1) checks every b-tree's structure, the
//! free list, that every record decodes, NOT NULL, CHECK and STRICT types,
//! and that every index that is not partial has as many entries as its table
//! has rows. It does not check that each row has an index entry with the
//! same key and rowid, UNIQUE constraints, the entry count of a partial
//! index, or values under a collation other than BINARY. A store that
//! passes it can still have an index that disagrees with its table without
//! changing the count (an entry under another key, or leading to another
//! row; a duplicated key), or a partial index that disagrees in any way: a
//! lost or misdirected page write on an SD card leaves exactly that. A
//! statement that finds rows through such an index acts on what the index
//! says, not on what the table holds.
//!
//! So startup reads each index whose disagreement could change what locks,
//! recovery or dispatch do, and holds it to the table's content, read by
//! the walk of `jobs` ([`super::JOB_WALK`]) or by a walk of the table: the
//! rows on each side are counted and summed as keyed hashes of their values
//! and rowid ([`Tally`]; a disagreement passes with probability 2^-64, the
//! keys are drawn afresh for every open); each index read whole is read as
//! stored and must be in order ([`index_tally`]): every entry after the one
//! before and, in a unique index, every key after the one before.
//!
//! - `jobs`: its primary key (`sqlite_autoindex_jobs_1`, which every keyed
//!   read of a job and every foreign key naming one uses, and which must be
//!   unique), its idempotency key (`sqlite_autoindex_jobs_2`, which keeps a
//!   retried request from queueing a second job), `jobs_state` (through
//!   which the lock invariants find the jobs holding locks, startup recovery
//!   finds the interrupted jobs, and the dispatcher finds the queued ones),
//!   and the partial `jobs_resource_lock` (one lock-holding job a primary
//!   resource). Read whole: they are small next to the table (at 5,113 jobs
//!   105, 210, about 40 and 1 pages against 2,054).
//! - `job_resources`, for the jobs not yet settled: the resource sets the
//!   lock invariants check and the next dispatch locks through the index
//!   (the acquisition trigger reads it). The table is walked for them.
//! - `resource_locks` and `job_resolutions`, with `PRAGMA integrity_check`
//!   on each: both are small (a lock per locked resource, a resolution per
//!   job an administrator settled), and it checks every row against every
//!   index of the table, UNIQUE included.
//!
//! Not accounted, and why: see `docs/soak.md`, "The table and its index".
use crate::{Error, ErrorCode, Result, durable};
use rusqlite::{Connection, Row, types::ValueRef};
use std::{
    collections::hash_map::RandomState,
    hash::{BuildHasher, Hasher},
};

/// States a job holds locks in (`jobs_resource_lock`'s and the lock
/// triggers' set).
pub(super) const LOCKING: [&str; 4] = [
    "running",
    "verifying",
    "outcome_unknown",
    "needs_intervention",
];
/// States a job no longer changes from: no lock, no dispatch.
pub(super) const SETTLED: [&str; 5] = [
    "succeeded",
    "failed",
    "canceled",
    "precondition_changed",
    "resolved",
];

/// The primary key's entries, as the index holds them (a plain scan of it:
/// no sorter, so the order read is the order stored).
pub(super) const ID_ENTRIES: &str = "SELECT id,rowid FROM jobs INDEXED BY sqlite_autoindex_jobs_1";
/// The idempotency key's entries, as the index holds them.
pub(super) const IDEMPOTENCY_ENTRIES: &str =
    "SELECT principal,idempotency,rowid FROM jobs INDEXED BY sqlite_autoindex_jobs_2";
/// `jobs_state`'s entries.
pub(super) const STATE_ENTRIES: &str =
    "SELECT state,deadline,rowid FROM jobs INDEXED BY jobs_state";
/// `jobs_resource_lock`'s entries, as the index holds them. Its condition is
/// the index's own, so SQLite reads the entries alone (the table cursor's
/// seek is deferred and never made).
pub(super) const LOCK_ENTRIES: &str = "SELECT resource,rowid FROM jobs INDEXED BY jobs_resource_lock WHERE state IN ('running','verifying','outcome_unknown','needs_intervention')";
/// The resource rows of the jobs listed (`?1`, the unsettled jobs), read
/// from the table.
pub(super) const UNSETTLED_RESOURCES: &str = "SELECT r.job,r.resource,r.rowid FROM job_resources r NOT INDEXED WHERE r.job IN (SELECT value FROM json_each(?1))";
/// The same jobs' entries in `job_resources`' primary key, found as every
/// reader finds them, by seeking each job: an entry the seek misses (out of
/// place, or under another key) is missing here, and an extra one is counted.
pub(super) const UNSETTLED_RESOURCE_ENTRIES: &str = "SELECT job,resource,rowid FROM job_resources INDEXED BY sqlite_autoindex_job_resources_1 WHERE job IN (SELECT value FROM json_each(?1))";
/// The lock tables, each checked against all its indexes.
pub(super) const LOCK_TABLES: [&str; 2] = [
    "PRAGMA integrity_check(resource_locks)",
    "PRAGMA integrity_check(job_resolutions)",
];

/// A multiset of rows: how many, and the wrapping sum of a keyed hash of
/// each row's values (with their types) and rowid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Tally {
    rows: u64,
    sum: u64,
}
impl Tally {
    fn add(&mut self, keys: &RandomState, values: &[ValueRef<'_>], rowid: i64) {
        let mut hasher = keys.build_hasher();
        for value in values {
            match value {
                ValueRef::Null => hasher.write_u8(0),
                ValueRef::Integer(n) => {
                    hasher.write_u8(1);
                    hasher.write_i64(*n);
                }
                ValueRef::Real(f) => {
                    hasher.write_u8(2);
                    hasher.write_u64(f.to_bits());
                }
                ValueRef::Text(bytes) => {
                    hasher.write_u8(3);
                    hasher.write_usize(bytes.len());
                    hasher.write(bytes);
                }
                ValueRef::Blob(bytes) => {
                    hasher.write_u8(4);
                    hasher.write_usize(bytes.len());
                    hasher.write(bytes);
                }
            }
        }
        hasher.write_i64(rowid);
        self.rows += 1;
        self.sum = self.sum.wrapping_add(hasher.finish());
    }
}

/// What the walk of `jobs` read, for the indexes of `jobs` and the
/// unsettled jobs' resource sets.
pub(super) struct Accounts {
    keys: RandomState,
    ids: Tally,
    idempotency: Tally,
    states: Tally,
    locks: Tally,
    unsettled: Vec<String>,
}
impl Accounts {
    pub(super) fn new() -> Self {
        Self {
            keys: RandomState::new(),
            ids: Tally::default(),
            idempotency: Tally::default(),
            states: Tally::default(),
            locks: Tally::default(),
            unsettled: Vec::new(),
        }
    }
    /// One row of the walk: `id, rowid, principal, idempotency, state,
    /// deadline, resource` in its first seven columns.
    pub(super) fn row(&mut self, row: &Row<'_>) -> Result<()> {
        let value = |n| row.get_ref(n).map_err(durable);
        let rowid: i64 = row.get(1).map_err(durable)?;
        let (id, state, resource) = (value(0)?, value(4)?, value(6)?);
        self.ids.add(&self.keys, &[id], rowid);
        self.idempotency
            .add(&self.keys, &[value(2)?, value(3)?], rowid);
        self.states.add(&self.keys, &[state, value(5)?], rowid);
        let state = state
            .as_str()
            .map_err(|_| Error(ErrorCode::StateNotDurable))?;
        if LOCKING.contains(&state) {
            self.locks.add(&self.keys, &[resource], rowid);
        }
        if !SETTLED.contains(&state) {
            self.unsettled.push(
                id.as_str()
                    .map_err(|_| Error(ErrorCode::StateNotDurable))?
                    .to_owned(),
            );
        }
        Ok(())
    }
    /// Refuses the store unless every accounted index agrees with what the
    /// walk read, every unique one is unique, the unsettled jobs' resource
    /// sets agree with their index, and the lock tables pass
    /// `integrity_check`.
    pub(super) fn verify(self, conn: &Connection) -> Result<()> {
        let keys = &self.keys;
        let entries = |sql: &str, columns: usize, order: Order| -> Result<Tally> {
            index_tally(conn, keys, sql, &[], columns, order)
        };
        let agree = self.ids == entries(ID_ENTRIES, 1, Order::UniqueKeys)?
            && self.idempotency == entries(IDEMPOTENCY_ENTRIES, 2, Order::UniqueKeys)?
            && self.states == entries(STATE_ENTRIES, 2, Order::Entries)?
            && self.locks == entries(LOCK_ENTRIES, 1, Order::UniqueKeys)?;
        if !agree {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        // With no unsettled job both sides are empty, known without reading
        // (the table side is a walk of `job_resources`, the largest cost of
        // the usual case, where every job has settled): skipped.
        if !self.unsettled.is_empty() {
            let unsettled = crate::json(&self.unsettled)?;
            let table = index_tally(
                conn,
                keys,
                UNSETTLED_RESOURCES,
                &[&unsettled],
                2,
                Order::Any,
            )?;
            let index = index_tally(
                conn,
                keys,
                UNSETTLED_RESOURCE_ENTRIES,
                &[&unsettled],
                2,
                Order::Any,
            )?;
            if table != index {
                return Err(Error(ErrorCode::StateNotDurable));
            }
        }
        for check in LOCK_TABLES {
            let mut statement = conn.prepare(check).map_err(durable)?;
            let found = statement
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(durable)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(durable)?;
            if found != ["ok"] {
                return Err(Error(ErrorCode::StateNotDurable));
            }
        }
        Ok(())
    }
}

/// What [`index_tally`] requires of the order rows come in.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Order {
    /// None: rows found by seeks, or a table's.
    Any,
    /// An index read whole, as stored: each entry (its key, then its rowid)
    /// strictly after the one before, under SQLite's ordering.
    Entries,
    /// A unique index read whole, as stored: each entry's key alone strictly
    /// after the one before (so no key twice either).
    UniqueKeys,
}

/// The tally of `sql`'s rows (`columns` values, then the rowid), refused
/// unless they come in `order`.
///
/// `quick_check` checks key order in table b-trees only, never in an index
/// (3.54.1: the "Rowid out of order" check is under `if( pPage->intKey )`).
/// An entry out of place keeps the index's multiset, so the tally passes,
/// but a seek, which binary-searches the index, misses it: a job filed out
/// of place in `jobs_state` is found by no `state=?` search. An index read
/// whole as stored with no sorter (the statements here), every entry after
/// the one before, is one in which every seek finds every entry.
pub(super) fn index_tally(
    conn: &Connection,
    keys: &RandomState,
    sql: &str,
    params: &[&dyn rusqlite::ToSql],
    columns: usize,
    order: Order,
) -> Result<Tally> {
    let mut statement = conn.prepare(sql).map_err(durable)?;
    let mut rows = statement.query(params).map_err(durable)?;
    let mut tally = Tally::default();
    let mut previous: Option<Vec<Key>> = None;
    while let Some(row) = rows.next().map_err(durable)? {
        let values = (0..columns)
            .map(|n| row.get_ref(n).map_err(durable))
            .collect::<Result<Vec<_>>>()?;
        let rowid: i64 = row.get(columns).map_err(durable)?;
        tally.add(keys, &values, rowid);
        if order == Order::Any {
            continue;
        }
        let mut entry = values
            .iter()
            .map(|v| Key::of(*v))
            .collect::<Result<Vec<_>>>()?;
        if order == Order::Entries {
            entry.push(Key::Integer(rowid));
        }
        if previous.as_ref().is_some_and(|before| *before >= entry) {
            return Err(Error(ErrorCode::StateNotDurable));
        }
        previous = Some(entry);
    }
    Ok(tally)
}

/// A key column's value, ordered as SQLite orders the types that occur in
/// these indexes under BINARY: NULL, then integers, then text by its bytes
/// (a prefix first), then blobs by theirs. A real is refused: no accounted
/// column holds one, and its order against integers is not modelled here.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Key {
    Null,
    Integer(i64),
    Text(Vec<u8>),
    Blob(Vec<u8>),
}
impl Key {
    fn of(value: ValueRef<'_>) -> Result<Self> {
        Ok(match value {
            ValueRef::Null => Self::Null,
            ValueRef::Integer(n) => Self::Integer(n),
            ValueRef::Text(bytes) => Self::Text(bytes.to_vec()),
            ValueRef::Blob(bytes) => Self::Blob(bytes.to_vec()),
            ValueRef::Real(_) => return Err(Error(ErrorCode::StateNotDurable)),
        })
    }
}
