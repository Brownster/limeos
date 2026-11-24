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
    // A store left at version `from`, with a receipt row, is migrated by
    // `open` and its schema then equals the reference. The store is built
    // with today's migration text, so this alone does not prove historical
    // compatibility; that rests on the text never having changed on the
    // mainline, and on a replay of the historical SQL (the plan's PR-43 note).
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
            difference(&rows(&store.conn).unwrap(), reference().unwrap()),
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
    assert_eq!(difference(&actual, reference().unwrap()), None);
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

#[test]
fn a_trigger_sharing_an_expected_name_cannot_hide_behind_it() {
    // Triggers have their own namespace: this one is valid beside the index
    // it is named after, and would delete every receipt as it is inserted.
    assert_eq!(
        refusal(
            "CREATE TRIGGER actions_resource_lock AFTER INSERT ON actions BEGIN DELETE FROM actions WHERE action=new.action; END;"
        ),
        "receipt store schema: unexpected trigger actions_resource_lock"
    );
    assert_eq!(
        refusal(
            "CREATE TRIGGER actions AFTER INSERT ON actions BEGIN DELETE FROM actions WHERE action=new.action; END;"
        ),
        "receipt store schema: unexpected trigger actions"
    );
    // A view takes an index's name only once the index is gone.
    assert_eq!(
        refusal("DROP INDEX actions_resource_lock; CREATE VIEW actions_resource_lock AS SELECT 1;"),
        "receipt store schema: index actions_resource_lock is missing"
    );
    // Tables, indexes and views share one namespace: an index cannot take a
    // table's name at all.
    let dir = private_dir();
    drop(ReceiptStore::open(dir.path()).unwrap());
    let conn = Connection::open(dir.path().join("receipts.sqlite")).unwrap();
    assert!(
        conn.execute_batch("CREATE INDEX retention ON actions(state);")
            .is_err()
    );
}

#[test]
fn a_store_after_analyze_still_opens() {
    let dir = private_dir();
    drop(ReceiptStore::open(dir.path()).unwrap());
    {
        let conn = Connection::open(dir.path().join("receipts.sqlite")).unwrap();
        conn.execute(
            "INSERT INTO actions(action,digest,resource,state,receipt) VALUES(?1,?2,'container:a','precondition_changed','{}')",
            ["a".repeat(64), "b".repeat(64)],
        )
        .unwrap();
        conn.execute_batch("ANALYZE;").unwrap();
    }
    let store = ReceiptStore::open_explained(dir.path())
        .unwrap_or_else(|r| panic!("refused after ANALYZE: {:?}", r.schema));
    let names: Vec<String> = rows(&store.conn)
        .unwrap()
        .into_iter()
        .map(|row| row.1)
        .filter(|name| name.starts_with("sqlite_stat"))
        .collect();
    assert!(names.contains(&"sqlite_stat1".to_string()), "{names:?}");
    // Exactly the statistics tables this SQLite creates are admitted.
    assert_eq!(
        reference()
            .unwrap()
            .statistics
            .iter()
            .map(|row| row.1.clone())
            .collect::<Vec<_>>(),
        names
    );
}

#[test]
fn a_forged_internal_object_is_still_refused() {
    // `sqlite_` names are reserved; only a forged schema row can carry one.
    // This also passed before statistics tables were admitted: it guards
    // that the admission stays exact, not a change it introduced.
    let dir = private_dir();
    drop(ReceiptStore::open(dir.path()).unwrap());
    {
        let conn = Connection::open(dir.path().join("receipts.sqlite")).unwrap();
        assert!(
            conn.execute_batch("CREATE VIEW sqlite_evil AS SELECT 1;")
                .is_err()
        );
        conn.execute_batch("PRAGMA writable_schema=ON; INSERT INTO sqlite_schema VALUES('view','sqlite_evil','sqlite_evil',0,'CREATE VIEW sqlite_evil AS SELECT 1'); PRAGMA writable_schema=OFF;").unwrap();
    }
    let refusal = ReceiptStore::open_explained(dir.path())
        .err()
        .expect("a forged internal object must be refused");
    assert_eq!(refusal.error, Error(ErrorCode::StateNotDurable));
    // SQLite may already refuse the forged row itself; if it loads, the
    // comparison names it.
    if let Some(schema) = refusal.schema {
        assert_eq!(
            schema.to_string(),
            "receipt store schema: unexpected view sqlite_evil"
        );
    }
}

#[test]
fn a_trigger_in_an_older_store_is_refused_before_any_migration_runs() {
    let dir = private_dir();
    let path = dir.path().join("receipts.sqlite");
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        migrate_to(&conn, 0, 5).unwrap();
        conn.execute(
            "INSERT INTO actions(action,digest,resource,state,receipt) VALUES(?1,?2,'container:a','precondition_changed','{}')",
            ["a".repeat(64), "b".repeat(64)],
        )
        .unwrap();
        // Version 6 updates every receipt; this would delete them all.
        conn.execute_batch(
            "CREATE TRIGGER wipe BEFORE UPDATE ON actions BEGIN DELETE FROM actions; END;",
        )
        .unwrap();
    }
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let refusal = ReceiptStore::open_explained(dir.path())
        .err()
        .expect("refused");
    assert_eq!(
        refusal.schema.unwrap().to_string(),
        "receipt store schema: unexpected trigger wipe"
    );
    let conn = Connection::open(&path).unwrap();
    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 5);
    let receipts: u32 = conn
        .query_row("SELECT count(*) FROM actions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(receipts, 1);
}

#[test]
fn a_foreign_key_action_in_an_older_store_cannot_delete_receipts_during_migration() {
    // A version-4 store whose stack table cascades deletes from `actions`.
    // Version 5 renames `actions` (SQLite follows the reference), copies it
    // and drops the old table; with foreign keys enforced that drop deleted
    // the stack row, and version 5 then rebuilt `stack_actions` from what
    // was left, so the version-9 comparison saw nothing wrong. The open
    // turns enforcement off: no version of the schema has a foreign key.
    let dir = private_dir();
    let path = dir.path().join("receipts.sqlite");
    let (action, digest) = ("a".repeat(64), "b".repeat(64));
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .unwrap();
        migrate_to(&conn, 0, 3).unwrap();
        conn.execute_batch("BEGIN IMMEDIATE; CREATE TABLE stack_actions(action TEXT PRIMARY KEY REFERENCES actions(action) ON DELETE CASCADE,digest TEXT NOT NULL,resource TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('prepared','effect_accepted','verified','precondition_changed','outcome_unknown','partly_done')),receipt TEXT NOT NULL,operation TEXT NOT NULL CHECK(operation IN ('restart','start','stop'))); CREATE UNIQUE INDEX stack_actions_lock ON stack_actions(resource) WHERE state IN ('prepared','effect_accepted','outcome_unknown'); PRAGMA user_version=4; COMMIT;").unwrap();
        conn.execute(
            "INSERT INTO actions(action,digest,resource,state,receipt) VALUES(?1,?2,'container:x','outcome_unknown','{}')",
            [&action, &digest],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO stack_actions(action,digest,resource,state,receipt,operation) VALUES(?1,?2,'stack:p','outcome_unknown','{}','restart')",
            [&action, &digest],
        )
        .unwrap();
    }
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    // Version 5 rebuilds both tables from its own text, so the store opens.
    let store = ReceiptStore::open_explained(dir.path())
        .unwrap_or_else(|r| panic!("refused: {:?}", r.schema));
    for table in ["actions", "stack_actions"] {
        let kept: u32 = store
            .conn
            .query_row(
                &format!(
                    "SELECT count(*) FROM {table} WHERE action=?1 AND state='outcome_unknown'"
                ),
                [&action],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kept, 1, "{table}");
    }
}
