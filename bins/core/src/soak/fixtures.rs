//! Stores in states normal traffic takes months to reach, or never reaches:
//! built through the store where its API can, and with plain SQL rows where
//! only the row count matters (session events, which nothing reads back).
//! Every fixture lives in the test's own temporary directory.
use super::support::START;
use limeos_domain::Principal;
use limeos_persistence::Store;
use rusqlite::{Connection, params};
use std::path::{Path, PathBuf};

/// A fresh core store with one enrolled administrator, closed again.
pub fn core_store(dir: &Path) -> (PathBuf, Principal) {
    let path = dir.join("core").join("core.sqlite");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut store = Store::open(&path).unwrap();
    store.issue_bootstrap("bootstrap", START).unwrap();
    store
        .enroll("bootstrap", "marc", "$argon2id$soak", START)
        .unwrap();
    let admin = store.login_record("marc").unwrap().unwrap().principal;
    (path, admin)
}

/// A writable connection to a closed store, for building fixtures.
pub fn writer(path: &Path) -> Connection {
    let conn = Connection::open(path).unwrap();
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    conn
}

/// Appends `rows` events of `kind` (a serialized `EventKind`) by `principal`,
/// for `job` if any, created at `created`, and keeps `meta.audit_bytes` equal
/// to its definition (`EVENT_BYTES` per row), as `Store::open` would.
pub fn add_events(
    path: &Path,
    principal: &str,
    job: Option<&str>,
    kind: &str,
    rows: i64,
    created: i64,
) {
    let mut conn = writer(path);
    let tx = conn.transaction().unwrap();
    tx.execute(
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<?1) INSERT INTO events(principal,job,kind,created) SELECT ?2,?3,?4,?5 FROM n",
        params![rows, principal, job, kind, created],
    )
    .unwrap();
    tx.execute(
        "UPDATE meta SET audit_bytes=(SELECT count(*) FROM events)*?1 WHERE id=1",
        [limeos_persistence::test_support::EVENT_BYTES],
    )
    .unwrap();
    tx.commit().unwrap();
}

/// A prunable event: job-less, not an automation proof, not authority.
pub const SESSION: &str = r#"{"kind":"session_created"}"#;

/// One integer from a store, read on a side connection.
pub fn value(path: &Path, sql: &str) -> i64 {
    super::checks::reader(path)
        .unwrap()
        .query_row(sql, [], |r| r.get(0))
        .unwrap()
}
