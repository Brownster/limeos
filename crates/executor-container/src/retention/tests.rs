use super::*;
use limeos_domain::{RestartPlan, StackPlan, StackSnapshot};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

fn private_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}
fn started(i: u32) -> String {
    format!(
        "2026-10-05T{:02}:{:02}:{:02}Z",
        7 + i / 3600,
        (i / 60) % 60,
        i % 60
    )
}
fn snapshot(c: char, i: u32, running: bool) -> ContainerSnapshot {
    ContainerSnapshot {
        resource: format!("container:{}", c.to_string().repeat(64)),
        image: format!("sha256:{}", "b".repeat(64)),
        started_at: started(i),
        running,
    }
}
fn ceiling() -> ContainerCeiling {
    ContainerCeiling {
        version: 1,
        core_uid: 1001,
        allow_restart: true,
        allow_start: true,
        allow_stop: true,
        allow_stack: true,
        managed_containers: ["a", "b", "c"].map(|c| c.repeat(64)).to_vec(),
        managed_stacks: vec!["media".into()],
    }
}
/// The plan of the i-th action: created at `1000 + 10 i`, valid for the TTL.
fn created(i: u32) -> i64 {
    1000 + 10 * i64::from(i)
}
fn restart(i: u32, expected: ContainerSnapshot) -> RestartRequest {
    let plan = RestartPlan {
        operation: ContainerAction::Restart,
        id: "c".repeat(64),
        version: 1,
        principal: "alice".into(),
        grant_revision: 1,
        expected,
        created_at: created(i),
        expires_at: created(i) + PLAN_TTL_SECONDS,
    };
    RestartRequest {
        action: format!("{i:064x}"),
        plan_digest: limeos_identity::digest(&encode(&plan).unwrap()),
        plan,
    }
}
/// A restart moves the start time forward, as a real Engine would.
struct Engine1 {
    current: Mutex<ContainerSnapshot>,
    effects: AtomicUsize,
}
impl Engine for Engine1 {
    async fn inspect(&self, _: &str) -> Result<ContainerSnapshot> {
        Ok(self.current.lock().unwrap().clone())
    }
    async fn restart(&self, _: &str) -> Result<()> {
        let n = self.effects.fetch_add(1, Ordering::SeqCst) as u32 + 1;
        self.current.lock().unwrap().started_at = started(n);
        Ok(())
    }
}
fn count(store: &ReceiptStore, sql: &str) -> u32 {
    store.conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

#[tokio::test]
async fn twenty_thousand_completed_actions_never_refuse_the_next_and_replay_stays_closed() {
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let engine = Engine1 {
        current: Mutex::new(snapshot('a', 0, true)),
        effects: AtomicUsize::new(0),
    };
    let total = 20_000;
    let mut first = None;
    let mut last = None;
    for i in 0..=total {
        let request = restart(i, engine.current.lock().unwrap().clone());
        let now = created(i) + 1;
        let receipt = store
            .execute(&engine, &ceiling(), 1001, &request, || now)
            .await
            .unwrap();
        assert_eq!(receipt.state, ExecutionState::EffectAccepted, "action {i}");
        let after = engine.current.lock().unwrap().clone();
        let verified = store
            .verify(
                &engine,
                &ceiling(),
                1001,
                &request.action,
                &request.plan_digest,
                &after,
            )
            .await
            .unwrap();
        assert_eq!(verified.state, ExecutionState::Verified);
        if i == 0 {
            first = Some(request);
        } else if i == total {
            last = Some(request);
        }
    }
    assert_eq!(engine.effects.load(Ordering::SeqCst), total as usize + 1);
    // Bounded: the retained set stays near the budget, never near the cap.
    let rows = count(&store, "SELECT count(*) FROM actions");
    assert!(rows <= RETAINED_ACTIONS + PRUNE_BATCH, "{rows} rows");
    assert!(rows >= RETAINED_ACTIONS, "{rows} rows");
    let now = created(total) + 1;
    // A retained id still deduplicates: the stored receipt, no second effect.
    let last = last.unwrap();
    let stored = store
        .execute(&engine, &ceiling(), 1001, &last, || now)
        .await
        .unwrap();
    assert_eq!(stored.state, ExecutionState::Verified);
    // A pruned id cannot execute again: its plan expired long before pruning.
    let first = first.unwrap();
    assert!(
        store
            .receipt(&first.action, &first.plan_digest)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store
            .execute(&engine, &ceiling(), 1001, &first, || now)
            .await
            .unwrap_err()
            .0,
        ErrorCode::Expired
    );
    assert_eq!(engine.effects.load(Ordering::SeqCst), total as usize + 1);
    // Retention never removes a receipt whose plan could still be presented.
    assert_eq!(
        count(
            &store,
            &format!(
                "SELECT count(*) FROM actions WHERE expires_at>{}",
                now - RETAIN_AFTER_EXPIRY
            )
        ),
        (RETAIN_AFTER_EXPIRY as u32 + PLAN_TTL_SECONDS as u32) / 10
    );
}

#[tokio::test]
async fn a_replayed_id_whose_receipt_was_pruned_is_refused_even_if_the_clock_steps_back() {
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let engine = Engine1 {
        current: Mutex::new(snapshot('a', 0, true)),
        effects: AtomicUsize::new(0),
    };
    let request = restart(0, engine.current.lock().unwrap().clone());
    store
        .execute(&engine, &ceiling(), 1001, &request, || created(0) + 1)
        .await
        .unwrap();
    let after = engine.current.lock().unwrap().clone();
    store
        .verify(
            &engine,
            &ceiling(),
            1001,
            &request.action,
            &request.plan_digest,
            &after,
        )
        .await
        .unwrap();
    store
        .conn
        .execute("DELETE FROM actions WHERE action=?", [&request.action])
        .unwrap();
    // The plan is current again only if the clock went back past its expiry;
    // its fresh precondition, which the first effect changed, still refuses.
    assert_eq!(
        store
            .execute(&engine, &ceiling(), 1001, &request, || created(0) + 2)
            .await
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    assert_eq!(engine.effects.load(Ordering::SeqCst), 1);
}

/// Replay protection covers a retained action id: the same id with any other
/// plan is `Conflict`. Once the receipt is pruned the id is unknown, and a new
/// valid plan under it executes like a fresh id would. That grants nothing: the
/// executor trusts core within the root-owned ceiling and does not verify
/// approvals, so core could mint a fresh id instead (ADR 0003). Pinned so a
/// change to it is deliberate.
#[tokio::test]
async fn a_pruned_action_id_with_a_new_valid_plan_executes_like_a_fresh_id() {
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let engine = Engine1 {
        current: Mutex::new(snapshot('a', 0, true)),
        effects: AtomicUsize::new(0),
    };
    let first = restart(0, engine.current.lock().unwrap().clone());
    store
        .execute(&engine, &ceiling(), 1001, &first, || created(0) + 1)
        .await
        .unwrap();
    let after = engine.current.lock().unwrap().clone();
    store
        .verify(
            &engine,
            &ceiling(),
            1001,
            &first.action,
            &first.plan_digest,
            &after,
        )
        .await
        .unwrap();
    // The same id, a later plan with fresh timestamps and its own digest.
    let later = 1000;
    let mut reused = restart(later, after.clone());
    reused.action = first.action.clone();
    let now = created(later) + 1;
    assert!(now > first.plan.expires_at + RETAIN_AFTER_EXPIRY);
    assert_eq!(
        store
            .execute(&engine, &ceiling(), 1001, &reused, || now)
            .await
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    assert_eq!(engine.effects.load(Ordering::SeqCst), 1);
    // Over budget with young receipts only: retention prunes the first one.
    for i in 0..RETAINED_ACTIONS {
        insert(
            &store,
            "actions",
            &format!("y{i}"),
            &format!("dy{i}"),
            &format!("container:y{i}"),
            "verified",
            now,
        );
    }
    assert_eq!(prune_all(&mut store, now), 1);
    assert!(
        store
            .receipt(&first.action, &first.plan_digest)
            .unwrap()
            .is_none()
    );
    let receipt = store
        .execute(&engine, &ceiling(), 1001, &reused, || now)
        .await
        .unwrap();
    assert_eq!(receipt.state, ExecutionState::EffectAccepted);
    assert_eq!(engine.effects.load(Ordering::SeqCst), 2);
}

fn insert(
    store: &ReceiptStore,
    table: &str,
    action: &str,
    digest: &str,
    resource: &str,
    state: &str,
    expires_at: i64,
) {
    let resolution = (state == "resolved").then_some("{}");
    store
        .conn
        .execute(
            &format!("INSERT INTO {table}(action,digest,resource,state,receipt,operation,resolution,expires_at) VALUES(?,?,?,?,'{{}}','restart',?,?)"),
            params![action, digest, resource, state, resolution, expires_at],
        )
        .unwrap();
}
fn prune_all(store: &mut ReceiptStore, now: i64) -> u32 {
    let mut total = 0;
    loop {
        let tx = store.conn.transaction().unwrap();
        let removed = prune(&tx, now).unwrap();
        // The documented bound holds even when stacks are removed whole.
        assert!(removed <= PRUNE_BATCH, "{removed} rows in one batch");
        tx.commit().unwrap();
        if removed == 0 {
            return total;
        }
        total += removed;
    }
}

#[test]
fn pruning_removes_only_old_terminal_receipts_and_whole_settled_stacks() {
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let old = 0;
    let now = 100_000;
    for i in 0..3000u32 {
        let state = if i % 2 == 0 {
            "verified"
        } else {
            "precondition_changed"
        };
        insert(
            &store,
            "actions",
            &format!("t{i}"),
            &format!("dt{i}"),
            &format!("container:t{i}"),
            state,
            old,
        );
    }
    // Never pruned: lock holders, operator resolutions, and anything whose
    // plan expired too recently.
    for (i, state) in ["prepared", "effect_accepted", "outcome_unknown", "resolved"]
        .iter()
        .enumerate()
    {
        insert(
            &store,
            "actions",
            &format!("keep{i}"),
            &format!("dk{i}"),
            &format!("container:k{i}"),
            state,
            old,
        );
    }
    for i in 0..10u32 {
        insert(
            &store,
            "actions",
            &format!("young{i}"),
            &format!("dy{i}"),
            &format!("container:y{i}"),
            "verified",
            now - RETAIN_AFTER_EXPIRY + 1,
        );
    }
    // 300 settled stacks with two members each, one stack with an unknown
    // member and one resolved stack.
    for i in 0..300u32 {
        let digest = format!("ds{i}");
        insert(
            &store,
            "stack_actions",
            &format!("s{i}"),
            &digest,
            &format!("stack:s{i}"),
            "partly_done",
            old,
        );
        insert(
            &store,
            "actions",
            &format!("s{i}m1"),
            &digest,
            &format!("container:s{i}m1"),
            "verified",
            old,
        );
        insert(
            &store,
            "actions",
            &format!("s{i}m2"),
            &digest,
            &format!("container:s{i}m2"),
            "precondition_changed",
            old,
        );
    }
    insert(
        &store,
        "stack_actions",
        "su",
        "dsu",
        "stack:su",
        "outcome_unknown",
        old,
    );
    insert(
        &store,
        "actions",
        "sum1",
        "dsu",
        "container:sum1",
        "outcome_unknown",
        old,
    );
    insert(
        &store,
        "stack_actions",
        "sr",
        "dsr",
        "stack:sr",
        "resolved",
        old,
    );
    insert(
        &store,
        "actions",
        "srm1",
        "dsr",
        "container:srm1",
        "resolved",
        old,
    );
    let before = count(&store, "SELECT count(*) FROM actions");
    let removed = prune_all(&mut store, now);
    assert_eq!(
        before - count(&store, "SELECT count(*) FROM actions")
            + count(&store, "SELECT 302 - count(*) FROM stack_actions"),
        removed
    );
    for kept in [
        "keep0", "keep1", "keep2", "keep3", "young0", "young9", "sum1", "srm1",
    ] {
        assert_eq!(
            count(
                &store,
                &format!("SELECT count(*) FROM actions WHERE action='{kept}'")
            ),
            1,
            "{kept}"
        );
    }
    assert_eq!(
        count(
            &store,
            "SELECT count(*) FROM stack_actions WHERE action IN ('su','sr')"
        ),
        2
    );
    // Exactly the budget remains, and the oldest went first.
    assert_eq!(
        count(&store, "SELECT count(*) FROM stack_actions"),
        RETAINED_STACKS
    );
    assert_eq!(
        count(
            &store,
            "SELECT count(*) FROM stack_actions WHERE action='s0'"
        ),
        0
    );
    assert_eq!(
        count(
            &store,
            "SELECT count(*) FROM stack_actions WHERE action='s299'"
        ),
        1
    );
    // No member outlives its stack; every retained stack keeps both members.
    assert_eq!(
        count(
            &store,
            "SELECT count(*) FROM actions a WHERE a.action LIKE 's%m%' AND NOT EXISTS (SELECT 1 FROM stack_actions s WHERE s.digest=a.digest)"
        ),
        0
    );
    assert_eq!(
        count(
            &store,
            "SELECT count(*) FROM actions a WHERE a.action LIKE 's%m%'"
        ),
        2 * (RETAINED_STACKS - 2) + 2
    );
    assert_eq!(
        count(
            &store,
            "SELECT count(*) FROM actions a WHERE NOT EXISTS (SELECT 1 FROM stack_actions s WHERE s.digest=a.digest)"
        ),
        RETAINED_ACTIONS
    );
    // Nothing over budget: a prune writes nothing.
    let changes = store.conn.total_changes();
    assert_eq!(prune_all(&mut store, now), 0);
    assert_eq!(store.conn.total_changes(), changes);
}

#[test]
fn no_receipt_is_pruned_while_core_still_reads_its_absence_as_never_sent() {
    // Settled stacks the executor verified and core never recorded, one more
    // than the budget, all with the same plan expiry.
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let expires = 50_000;
    for i in 0..=RETAINED_STACKS {
        let digest = format!("dv{i}");
        insert(
            &store,
            "stack_actions",
            &format!("v{i}"),
            &digest,
            &format!("stack:v{i}"),
            "verified",
            expires,
        );
        insert(
            &store,
            "actions",
            &format!("v{i}m"),
            &digest,
            &format!("container:v{i}m"),
            "verified",
            expires,
        );
    }
    // Core's window, and the retention after it, prune nothing.
    let last_unsent = expires + limeos_domain::RECEIPT_RETAIN_AFTER_EXPIRY
        - limeos_domain::RECEIPT_ABSENCE_MARGIN
        - 1;
    assert!(limeos_domain::receipt_absence_proves_unsent(
        expires,
        last_unsent
    ));
    assert_eq!(prune_all(&mut store, last_unsent), 0);
    assert_eq!(prune_all(&mut store, expires + RETAIN_AFTER_EXPIRY - 1), 0);
    // The executor's earliest pruning is outside core's window.
    let first_pruned = expires + RETAIN_AFTER_EXPIRY;
    assert_eq!(prune_all(&mut store, first_pruned), 2);
    assert!(!limeos_domain::receipt_absence_proves_unsent(
        expires,
        first_pruned
    ));
    assert_eq!(
        count(
            &store,
            "SELECT count(*) FROM stack_actions WHERE action='v0'"
        ),
        0
    );
}

#[test]
fn a_batch_stops_before_a_stack_that_would_exceed_the_row_bound() {
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let members = limeos_domain::MAX_STACK_MEMBERS as u32;
    for i in 0..RETAINED_STACKS + 10 {
        let digest = format!("dw{i}");
        insert(
            &store,
            "stack_actions",
            &format!("w{i}"),
            &digest,
            &format!("stack:w{i}"),
            "verified",
            0,
        );
        for m in 0..members {
            insert(
                &store,
                "actions",
                &format!("w{i}m{m}"),
                &digest,
                &format!("container:w{i}m{m}"),
                "verified",
                0,
            );
        }
    }
    let tx = store.conn.transaction().unwrap();
    let removed = prune(&tx, 100_000).unwrap();
    tx.commit().unwrap();
    // Whole stacks of 17 rows: three fit in 64, a fourth would not.
    assert_eq!(removed, (PRUNE_BATCH / (members + 1)) * (members + 1));
    assert_eq!(
        count(&store, "SELECT count(*) FROM stack_actions"),
        RETAINED_STACKS + 10 - PRUNE_BATCH / (members + 1)
    );
    assert_eq!(
        prune_all(&mut store, 100_000),
        (10 - PRUNE_BATCH / (members + 1)) * (members + 1)
    );
    assert_eq!(
        count(&store, "SELECT count(*) FROM stack_actions"),
        RETAINED_STACKS
    );
}

/// Stack actions every 100 seconds, so the count budget, not the expiry
/// margin, decides what is kept.
fn stack_created(i: u32) -> i64 {
    1000 + 100 * i64::from(i)
}
fn stack_request(i: u32) -> StackRequest {
    let members = vec![snapshot('a', 0, true), snapshot('b', 0, true)];
    let plan = StackPlan {
        id: "c".repeat(64),
        version: limeos_domain::STACK_VERSION,
        operation: ContainerAction::Restart,
        principal: "alice".into(),
        grant_revision: 1,
        catalog_stack: "registered".into(),
        expected: StackSnapshot::new("media", members).unwrap(),
        created_at: stack_created(i),
        expires_at: stack_created(i) + PLAN_TTL_SECONDS,
    };
    StackRequest {
        action: format!("{i:064x}"),
        plan_digest: limeos_identity::digest(&encode(&plan).unwrap()),
        plan,
    }
}
/// The listed project differs from every plan, so each action is refused
/// before any effect and leaves a settled stack receipt with member rows.
struct Drifted;
impl Engine for Drifted {
    async fn inspect(&self, _: &str) -> Result<ContainerSnapshot> {
        Err(Error(ErrorCode::NotFound))
    }
    async fn restart(&self, _: &str) -> Result<()> {
        panic!("no effect");
    }
    async fn project_members(&self, _: &str) -> Result<Vec<ContainerSnapshot>> {
        Ok(vec![snapshot('a', 5, true), snapshot('b', 5, true)])
    }
}

#[tokio::test]
async fn settled_stack_receipts_are_pruned_with_their_members_and_retained_ones_still_cross_check()
{
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let total = 1000;
    for i in 0..total {
        let request = stack_request(i);
        let receipt = store
            .execute_stack(&Drifted, &ceiling(), 1001, &request, || {
                stack_created(i) + 1
            })
            .await
            .unwrap();
        assert_eq!(receipt.state, ExecutionState::PreconditionChanged);
    }
    let stacks = count(&store, "SELECT count(*) FROM stack_actions");
    assert!(stacks <= RETAINED_STACKS + 1, "{stacks}");
    assert_eq!(count(&store, "SELECT count(*) FROM actions"), 2 * stacks);
    let last = stack_request(total - 1);
    assert!(
        store
            .stack_receipt(&last.action, &last.plan_digest)
            .unwrap()
            .is_some()
    );
    let first = stack_request(0);
    assert!(
        store
            .stack_receipt(&first.action, &first.plan_digest)
            .unwrap()
            .is_none()
    );
}

#[test]
fn prune_kill_helper() {
    let Some(directory) = std::env::var_os("LIMEOS_RECEIPT_PRUNE_FIXTURE") else {
        return;
    };
    let mut store = ReceiptStore::open(Path::new(&directory)).unwrap();
    let tx = store.conn.transaction().unwrap();
    assert!(prune(&tx, 100_000).unwrap() > 0);
    // Killed with the deletions written but not committed.
    std::process::abort();
}
#[test]
fn a_process_killed_mid_prune_leaves_a_store_that_opens_with_every_row() {
    let dir = private_dir();
    {
        let store = ReceiptStore::open(dir.path()).unwrap();
        for i in 0..(RETAINED_ACTIONS + 500) {
            insert(
                &store,
                "actions",
                &format!("t{i}"),
                &format!("dt{i}"),
                &format!("container:t{i}"),
                "verified",
                0,
            );
        }
    }
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "retention::tests::prune_kill_helper",
            "--nocapture",
        ])
        .env("LIMEOS_RECEIPT_PRUNE_FIXTURE", dir.path())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(!status.success());
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    assert_eq!(
        count(&store, "SELECT count(*) FROM actions"),
        RETAINED_ACTIONS + 500
    );
    assert!(prune_all(&mut store, 100_000) > 0);
    assert_eq!(
        count(&store, "SELECT count(*) FROM actions"),
        RETAINED_ACTIONS
    );
}

#[test]
fn schema_five_upgrade_keeps_receipts_and_retains_them_for_a_plan_lifetime() {
    let dir = private_dir();
    let store = ReceiptStore::open(dir.path()).unwrap();
    insert(
        &store,
        "actions",
        "old",
        "dold",
        "container:old",
        "verified",
        0,
    );
    store
        .conn
        .execute_batch("DROP INDEX actions_digest; DROP INDEX stack_actions_digest; ALTER TABLE actions DROP COLUMN expires_at; ALTER TABLE stack_actions DROP COLUMN expires_at; PRAGMA user_version=5;")
        .unwrap();
    drop(store);
    let store = ReceiptStore::open(dir.path()).unwrap();
    let version: u32 = store
        .conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, crate::RECEIPT_SCHEMA_VERSION);
    let wall = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let expires: i64 = store
        .conn
        .query_row(
            "SELECT expires_at FROM actions WHERE action='old'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!((wall + PLAN_TTL_SECONDS - 5..=wall + PLAN_TTL_SECONDS + 5).contains(&expires));
}
