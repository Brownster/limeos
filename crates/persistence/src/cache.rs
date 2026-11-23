//! Page-cache sizes of the store's two long-lived connections.
//!
//! SQLite's default cache is 2000 KiB a connection, and a connection keeps what it
//! has filled for as long as it is open, which for core is months. The startup
//! `quick_check` reads every page of the store, so with the default each
//! connection would hold up to 2000 KiB from the first minute on a store that
//! has grown, whatever it does afterwards. Each connection gets a cache sized to
//! its work instead (the page size is SQLite's default, 4096 bytes):
//!
//! * The writer runs every write and also the listing and job-history reads. A
//!   cache too small for a transaction's dirty pages spills them into the WAL
//!   before commit, and a page changed again after its spill is written again,
//!   so `WRITER_KIB` holds retention's normal transactions with room to spare.
//!   Measured with the bundled SQLite (`retention::tests`): a full event batch
//!   (1,024 events) dirties 67 pages densely packed and 272 with one prunable
//!   event in four among pinned history; a day's job-row batch and startup's
//!   `make_room` at the cap stay well inside; none spills at 1536 KiB (384
//!   pages). A backlog job-row batch (64 jobs, up to 2,048 rows scattered)
//!   dirtied 442 pages and spilled 437 KB at this size and 21 KB at the 2000 KiB
//!   default: it runs only while a backlog of settled jobs past their horizon
//!   lasts. An upgrade runs on its own exclusive connection with SQLite's
//!   default (a migration rewrites whole tables), and that connection closes
//!   when the upgrade ends; the normal connection opened after it is set to
//!   `WRITER_KIB`. An open with no upgrade sets it before its startup check,
//!   so the check reads through the small cache, and again once startup has
//!   committed (`settle`).
//! * The reader answers session and revocation checks only: a few B-tree paths
//!   of the sessions and task-token tables. A cache smaller than a query's
//!   pages costs re-reads from the kernel's page cache, never writes, so
//!   `READER_KIB` is small.
use crate::{Result, durable};
use rusqlite::Connection;

pub(crate) const WRITER_KIB: u32 = 1536;
pub(crate) const READER_KIB: u32 = 256;

/// Sets the connection's cache to `kib` and frees whatever it holds that is not
/// in use (`shrink_memory`). Outside a transaction.
pub(crate) fn settle(conn: &Connection, kib: u32) -> Result<()> {
    conn.execute_batch(&format!("PRAGMA cache_size=-{kib}; PRAGMA shrink_memory;"))
        .map_err(durable)
}
