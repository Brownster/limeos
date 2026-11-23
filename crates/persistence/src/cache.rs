//! Page-cache sizes of the store's two connections.
//!
//! SQLite's default cache is 2000 KiB a connection, and a connection keeps what it
//! has filled for as long as it is open, which for core is months. The startup
//! `quick_check` reads every page of the store, so with the default each
//! connection would hold up to 2000 KiB from the first minute on a store that
//! has grown, whatever it does afterwards. Each connection gets a cache sized to
//! its work instead:
//!
//! * The writer's largest transaction in normal operation is a retention pass:
//!   at most `EVENT_BATCH` events with their index entries, and smaller batches
//!   of occurrences and authority events. A cache too small for a transaction's
//!   dirty pages spills them into the WAL before commit (extra writes, the same
//!   pages written twice), so `WRITER_KIB` holds a full event batch with room to
//!   spare: the test `a_full_retention_batch_writes_no_more_with_the_small_cache`
//!   finds the same WAL bytes at this size as at the default. A migration keeps
//!   the default for its own transaction (an upgrade rewrites whole tables) and
//!   the small cache is set once it has committed.
//! * The reader only reads: lists, a job's history, cursors. A cache smaller
//!   than a query's pages costs re-reads from the kernel's page cache, never
//!   writes, so `READER_KIB` covers a few B-tree paths and the pages of a page of
//!   results.
use crate::{Result, durable};
use rusqlite::Connection;

pub(crate) const WRITER_KIB: u32 = 1024;
pub(crate) const READER_KIB: u32 = 512;

/// Sets the connection's cache to `kib` and gives back whatever it holds beyond
/// what it is using. Outside a transaction.
pub(crate) fn settle(conn: &Connection, kib: u32) -> Result<()> {
    conn.execute_batch(&format!("PRAGMA cache_size=-{kib}; PRAGMA shrink_memory;"))
        .map_err(durable)
}
