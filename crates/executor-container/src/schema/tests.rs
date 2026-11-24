use super::*;
use crate::{ReceiptStore, migrate_to};
use std::os::unix::fs::PermissionsExt;

fn private_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}
/// A fresh store, changed by `sql` behind the store's back, then reopened.
fn altered(sql: &str) -> std::result::Result<ReceiptStore, Refusal> {
    let dir = private_dir();
    drop(ReceiptStore::open(dir.path()).unwrap());
    {
        let conn = Connection::open(dir.path().join("receipts.sqlite")).unwrap();
        conn.execute_batch(sql).unwrap();
        // Every case below survives SQLite's own structural check.
        assert_eq!(
            conn.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
    }
    let result = ReceiptStore::open_explained(dir.path());
    // `open` answers the same refusal with the error alone.
    if result.is_err() {
        assert_eq!(
            ReceiptStore::open(dir.path()).err(),
            Some(Error(ErrorCode::StateNotDurable))
        );
    }
    result
}
fn refusal(sql: &str) -> String {
    let refusal = altered(sql)
        .err()
        .expect("the altered store must be refused");
    assert_eq!(refusal.error, Error(ErrorCode::StateNotDurable));
    refusal.schema.expect("named schema object").to_string()
}

#[test]
fn a_store_left_at_each_earlier_version_migrates_and_opens() {
    // A store an earlier release left at version `from`, with a receipt row
    // where the schema has a table for it, is migrated by `open` and its
    // schema then equals the reference.
    for from in 1..crate::RECEIPT_SCHEMA_VERSION {
        let dir = private_dir();
        {
            let conn = Connection::open(dir.path().join("receipts.sqlite")).unwrap();
            conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
            migrate_to(&conn, 0, from).unwrap();
            conn.execute(
                "INSERT INTO actions(action,digest,resource,state,receipt) VALUES(?1,?2,'container:a','precondition_changed','{}')",
                ["a".repeat(64), "b".repeat(64)],
            )
            .unwrap();
            let version: u32 = conn
                .query_row("PRAGMA user_version", [], |r| r.get(0))
                .unwrap();
            assert_eq!(version, from);
        }
        // As every store `open` created: private to its owner.
        std::fs::set_permissions(
            dir.path().join("receipts.sqlite"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let store = ReceiptStore::open_explained(dir.path())
            .unwrap_or_else(|r| panic!("store from version {from} refused: {:?}", r.schema));
        assert_eq!(
            difference(&rows(&store.conn).unwrap(), &reference().unwrap()),
            None
        );
        drop(store);
        // And again, as every later start opens it.
        ReceiptStore::open(dir.path()).unwrap();
    }
}

#[test]
fn a_fresh_store_holds_exactly_the_reference_and_its_lock_indexes() {
    let dir = private_dir();
    let store = ReceiptStore::open(dir.path()).unwrap();
    let actual = rows(&store.conn).unwrap();
    assert_eq!(difference(&actual, &reference().unwrap()), None);
    for (name, sql) in [
        (
            "actions_resource_lock",
            "CREATE UNIQUE INDEX actions_resource_lock ON actions(resource) WHERE state IN ('prepared','effect_accepted','outcome_unknown')",
        ),
        (
            "stack_actions_lock",
            "CREATE UNIQUE INDEX stack_actions_lock ON stack_actions(resource) WHERE state IN ('prepared','effect_accepted','outcome_unknown')",
        ),
    ] {
        assert!(
            actual
                .iter()
                .any(|row| row.1 == name && row.3.as_deref() == Some(sql)),
            "{name}"
        );
    }
    assert!(!actual.iter().any(|row| row.0 == "trigger"));
}

#[test]
fn a_missing_resource_lock_index_refuses_naming_it() {
    assert_eq!(
        refusal("DROP INDEX actions_resource_lock;"),
        "receipt store schema: index actions_resource_lock is missing"
    );
    assert_eq!(
        refusal("DROP INDEX stack_actions_lock;"),
        "receipt store schema: index stack_actions_lock is missing"
    );
    assert_eq!(
        refusal("DROP INDEX actions_digest;"),
        "receipt store schema: index actions_digest is missing"
    );
    assert_eq!(
        refusal("DROP INDEX effect_log_resource;"),
        "receipt store schema: index effect_log_resource is missing"
    );
}

#[test]
fn a_lock_index_without_unique_or_with_another_condition_refuses() {
    assert_eq!(
        refusal(
            "DROP INDEX actions_resource_lock; CREATE INDEX actions_resource_lock ON actions(resource) WHERE state IN ('prepared','effect_accepted','outcome_unknown');"
        ),
        "receipt store schema: index actions_resource_lock differs from the definition schema 9 expects"
    );
    assert_eq!(
        refusal(
            "DROP INDEX actions_resource_lock; CREATE UNIQUE INDEX actions_resource_lock ON actions(resource) WHERE state IN ('prepared');"
        ),
        "receipt store schema: index actions_resource_lock differs from the definition schema 9 expects"
    );
    assert_eq!(
        refusal(
            "DROP INDEX stack_actions_lock; CREATE UNIQUE INDEX stack_actions_lock ON stack_actions(resource,state) WHERE state IN ('prepared','effect_accepted','outcome_unknown');"
        ),
        "receipt store schema: index stack_actions_lock differs from the definition schema 9 expects"
    );
}

#[test]
fn an_unexpected_trigger_or_index_refuses_naming_it() {
    assert_eq!(
        refusal(
            "CREATE TRIGGER forget AFTER INSERT ON actions BEGIN UPDATE actions SET state='verified' WHERE action=new.action; END;"
        ),
        "receipt store schema: unexpected trigger forget"
    );
    assert_eq!(
        refusal("CREATE INDEX extra ON actions(state);"),
        "receipt store schema: unexpected index extra"
    );
}

#[test]
fn a_table_rebuilt_without_its_constraints_refuses_naming_it() {
    assert_eq!(
        refusal(
            "DROP TABLE retention; CREATE TABLE retention(id INTEGER PRIMARY KEY,watermark INTEGER NOT NULL); INSERT INTO retention VALUES(1,0);"
        ),
        "receipt store schema: table retention differs from the definition schema 9 expects"
    );
    // The action id's uniqueness is the primary key's automatic index.
    let detail = refusal(
        "ALTER TABLE pruned_legacy RENAME TO old; CREATE TABLE pruned_legacy(action TEXT); INSERT INTO pruned_legacy SELECT action FROM old; DROP TABLE old;",
    );
    assert_eq!(
        detail,
        "receipt store schema: table pruned_legacy differs from the definition schema 9 expects"
    );
}
