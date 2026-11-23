use super::*;
use limeos_domain::{ActionPlan, RestartPlan, StackPlan, StackSnapshot};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

/// A store the tests create has pruned nothing (`initial_watermark`).
const FRESH: i64 = i64::MIN;

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
    // Twenty thousand restarts of one container, ten seconds apart: far past
    // its effect budget (`limit`), which is not what this test is about.
    let mut store = ReceiptStore::open(dir.path()).unwrap().unlimited();
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
    // A pruned id cannot execute again: its plan expires at or below the
    // watermark, so it is refused before any clock is read (and it expired
    // long before pruning besides).
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
        ErrorCode::BeyondReceiptHistory
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
    // Never pruned: lock holders, and anything whose plan expired too recently.
    for (i, state) in ["prepared", "effect_accepted", "outcome_unknown"]
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
    // 300 settled stacks with two members each and one stack with an unknown
    // member.
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
    let before = count(&store, "SELECT count(*) FROM actions");
    let removed = prune_all(&mut store, now);
    assert_eq!(
        before - count(&store, "SELECT count(*) FROM actions")
            + count(&store, "SELECT 301 - count(*) FROM stack_actions"),
        removed
    );
    for kept in ["keep0", "keep1", "keep2", "young0", "young9", "sum1"] {
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
            "SELECT count(*) FROM stack_actions WHERE action='su'"
        ),
        1
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
        2 * (RETAINED_STACKS - 1) + 1
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
fn resolved_receipts_are_pruned_as_settled_history_and_lock_holders_never_are() {
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    // Standalone: five resolved receipts, oldest of the settled ones; three
    // lock holders, older still; and settled history up to the budget.
    for i in 0..5u32 {
        insert(
            &store,
            "actions",
            &id('a', i),
            &format!("da{i}"),
            &format!("container:a{i}"),
            "resolved",
            100 + i64::from(i),
        );
    }
    for (i, state) in ["prepared", "effect_accepted", "outcome_unknown"]
        .iter()
        .enumerate()
    {
        insert(
            &store,
            "actions",
            &id('b', i as u32),
            &format!("db{i}"),
            &format!("container:b{i}"),
            state,
            50,
        );
    }
    for i in 0..RETAINED_ACTIONS {
        insert(
            &store,
            "actions",
            &id('c', i),
            &format!("dc{i}"),
            &format!("container:c{i}"),
            "verified",
            1_000 + i64::from(i),
        );
    }
    // Stacks: a resolved stack whose members were resolved or already
    // terminal, oldest first, then settled stacks up to the budget.
    insert(
        &store,
        "stack_actions",
        &id('5', 0),
        "dr",
        "stack:r",
        "resolved",
        90,
    );
    insert(
        &store,
        "actions",
        &id('7', 0),
        "dr",
        "container:r0",
        "resolved",
        90,
    );
    insert(
        &store,
        "actions",
        &id('7', 1),
        "dr",
        "container:r1",
        "precondition_changed",
        90,
    );
    for i in 1..=RETAINED_STACKS {
        settled_stack(&store, i, 2_000 + i64::from(i));
    }
    let now = 100_000;
    prune_all(&mut store, now);
    // Every resolved row went, the resolved stack with both its members.
    assert_eq!(
        count(
            &store,
            "SELECT count(*) FROM actions WHERE state='resolved'"
        ),
        0
    );
    assert_eq!(
        count(
            &store,
            "SELECT count(*) FROM stack_actions WHERE state='resolved'"
        ),
        0
    );
    assert_eq!(
        count(&store, "SELECT count(*) FROM actions WHERE digest='dr'"),
        0
    );
    // Lock holders stay, whatever their age.
    assert_eq!(
        count(
            &store,
            "SELECT count(*) FROM actions WHERE state IN ('prepared','effect_accepted','outcome_unknown')"
        ),
        3
    );
    assert_eq!(
        count(&store, "SELECT count(*) FROM stack_actions"),
        RETAINED_STACKS
    );
    // The watermark covers them: absence proves nothing about a resolved
    // action whose receipt is gone.
    for i in 0..5 {
        assert_eq!(
            absence(
                &store,
                ResolveKind::Container,
                &id('a', i),
                100 + i64::from(i)
            ),
            ReceiptAbsence::NoLongerHeld
        );
        assert!(
            store
                .resolution(ResolveKind::Container, &id('a', i))
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(
        absence(&store, ResolveKind::Stack, &id('5', 0), 90),
        ReceiptAbsence::NoLongerHeld
    );
    assert!(watermark_of(&store) >= 1_000);
}

fn watermark_of(store: &ReceiptStore) -> i64 {
    watermark(&store.conn).unwrap()
}
fn absence(
    store: &ReceiptStore,
    kind: ResolveKind,
    action: &str,
    expires_at: i64,
) -> ReceiptAbsence {
    store.absence_at(kind, action, expires_at).unwrap()
}
fn id(tag: char, i: u32) -> String {
    format!("{tag}{i:063x}")
}
/// One settled stack with one member, both rows at `expires_at`.
fn settled_stack(store: &ReceiptStore, i: u32, expires_at: i64) {
    let digest = format!("ds{i}");
    insert(
        store,
        "stack_actions",
        &id('5', i),
        &digest,
        &format!("stack:s{i}"),
        "verified",
        expires_at,
    );
    insert(
        store,
        "actions",
        &id('6', i),
        &digest,
        &format!("container:s{i}"),
        "verified",
        expires_at,
    );
}

#[test]
fn a_pruned_stack_is_no_longer_held_whatever_the_clock_does_and_a_new_plan_is_never_recorded() {
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    // Nothing pruned yet: any plan without a row was never recorded.
    assert_eq!(watermark_of(&store), FRESH);
    assert_eq!(
        absence(&store, ResolveKind::Stack, &id('5', 0), 0),
        ReceiptAbsence::NeverRecorded
    );
    // One stack over budget, all plans expiring at 50_000, the oldest first.
    let expires = 50_000;
    for i in 0..=RETAINED_STACKS {
        settled_stack(&store, i, expires + i64::from(i));
    }
    // Nothing is pruned until a plan has been expired for the retention hour.
    assert_eq!(prune_all(&mut store, expires + RETAIN_AFTER_EXPIRY - 1), 0);
    assert_eq!(watermark_of(&store), FRESH);
    let pruned_at = expires + RETAIN_AFTER_EXPIRY;
    assert_eq!(prune_all(&mut store, pruned_at), 2);
    assert_eq!(watermark_of(&store), expires);
    // The executed and pruned stack: absence proves nothing, and the answer
    // does not read the clock, so stepping it back an hour changes nothing.
    for kind in [ResolveKind::Stack, ResolveKind::Container] {
        let action = if kind == ResolveKind::Stack {
            id('5', 0)
        } else {
            id('6', 0)
        };
        assert_eq!(
            absence(&store, kind, &action, expires),
            ReceiptAbsence::NoLongerHeld
        );
    }
    // Another plan with the same expiry may have been pruned too: equality errs
    // towards "no longer held".
    assert_eq!(
        absence(&store, ResolveKind::Stack, &id('7', 0), expires),
        ReceiptAbsence::NoLongerHeld
    );
    // A plan expiring after the watermark was never recorded, at once.
    assert_eq!(
        absence(&store, ResolveKind::Stack, &id('7', 0), expires + 1),
        ReceiptAbsence::NeverRecorded
    );
    // A row that exists is looked up, not judged.
    assert_eq!(
        store
            .absence_at(ResolveKind::Stack, &id('5', 1), expires)
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    // Durable across a restart.
    drop(store);
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    assert_eq!(watermark_of(&store), expires);
    // Never decreases: a row with an older expiry pruned later, at a clock that
    // stepped back, leaves the watermark where it was.
    store
        .conn
        .execute(
            &format!(
                "UPDATE stack_actions SET expires_at={} WHERE action='{}'",
                expires - 10_000,
                id('5', 1)
            ),
            [],
        )
        .unwrap();
    store
        .conn
        .execute(
            &format!(
                "UPDATE actions SET expires_at={} WHERE action='{}'",
                expires - 10_000,
                id('6', 1)
            ),
            [],
        )
        .unwrap();
    settled_stack(&store, 9_999, expires + 9_999);
    assert_eq!(prune_all(&mut store, expires - 1_000), 2);
    assert_eq!(watermark_of(&store), expires);
    assert_eq!(
        absence(&store, ResolveKind::Stack, &id('5', 1), expires - 10_000),
        ReceiptAbsence::NoLongerHeld
    );
}

#[test]
fn a_missing_receipt_is_judged_by_the_carried_plan_and_a_forged_expiry_is_refused() {
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    // The action ran with a plan expiring at `w`, settled, and was pruned.
    let ran = restart(0, snapshot('a', 0, true));
    let w = ran.plan.expires_at;
    insert(
        &store,
        "actions",
        &ran.action,
        &ran.plan_digest,
        &ran.plan.expected.resource,
        "verified",
        w,
    );
    for i in 1..=RETAINED_ACTIONS {
        insert(
            &store,
            "actions",
            &id('b', i),
            &format!("db{i}"),
            &format!("container:b{i}"),
            "verified",
            w + 10 + i64::from(i),
        );
    }
    assert_eq!(prune_all(&mut store, w + RETAIN_AFTER_EXPIRY), 1);
    assert_eq!(watermark_of(&store), w);
    let plan = ActionPlan::Container(ran.plan.clone());
    assert_eq!(
        store
            .receipt_absence(&ran.action, &ran.plan_digest, &plan)
            .unwrap(),
        ReceiptAbsence::NoLongerHeld
    );
    // A bare expiry one second later would have been judged never recorded.
    assert_eq!(
        store
            .absence_at(ResolveKind::Container, &ran.action, w + 1)
            .unwrap(),
        ReceiptAbsence::NeverRecorded
    );
    // A peer naming the same action and plan digest with a plan that expires
    // one second later is refused: that plan does not hash to the digest.
    let mut forged = ran.plan.clone();
    forged.created_at += 1;
    forged.expires_at += 1;
    let forged = ActionPlan::Container(forged);
    assert_eq!(
        store
            .receipt_absence(&ran.action, &ran.plan_digest, &forged)
            .unwrap_err()
            .0,
        ErrorCode::InvalidInput
    );
    // So is a plan of the other kind, and a malformed plan hashed honestly.
    assert_eq!(
        verified_expiry(ResolveKind::Stack, &ran.plan_digest, &plan)
            .unwrap_err()
            .0,
        ErrorCode::InvalidInput
    );
    let mut malformed = ran.plan.clone();
    malformed.created_at = malformed.expires_at;
    let digest = limeos_identity::digest(&encode(&malformed).unwrap());
    assert_eq!(
        store
            .receipt_absence(&ran.action, &digest, &ActionPlan::Container(malformed))
            .unwrap_err()
            .0,
        ErrorCode::InvalidInput
    );
    assert_eq!(
        verified_expiry(ResolveKind::Container, &ran.plan_digest, &plan).unwrap(),
        w
    );
}

#[test]
fn the_watermark_covers_standalone_receipts_and_moves_in_the_pruning_transaction() {
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    for i in 0..RETAINED_ACTIONS + 3 {
        insert(
            &store,
            "actions",
            &id('a', i),
            &format!("da{i}"),
            &format!("container:a{i}"),
            "verified",
            1_000 + i64::from(i),
        );
    }
    // A pruning transaction that is rolled back leaves the watermark too.
    {
        let tx = store.conn.transaction().unwrap();
        assert_eq!(prune(&tx, 100_000).unwrap(), 3);
        tx.rollback().unwrap();
    }
    assert_eq!(watermark_of(&store), FRESH);
    assert_eq!(prune_all(&mut store, 100_000), 3);
    assert_eq!(watermark_of(&store), 1_002);
    for i in 0..3 {
        assert_eq!(
            absence(
                &store,
                ResolveKind::Container,
                &id('a', i),
                1_000 + i64::from(i)
            ),
            ReceiptAbsence::NoLongerHeld
        );
    }
    assert_eq!(
        absence(&store, ResolveKind::Container, &id('b', 0), 1_003),
        ReceiptAbsence::NeverRecorded
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
    // The watermark moves with the deletions or not at all.
    assert_eq!(watermark_of(&store), FRESH);
    assert!(prune_all(&mut store, 100_000) > 0);
    assert_eq!(watermark_of(&store), 0);
    assert_eq!(
        count(&store, "SELECT count(*) FROM actions"),
        RETAINED_ACTIONS
    );
}

/// Turn a fresh store back into schema 5 or 6, keeping its rows.
fn downgrade(store: ReceiptStore, to: u32) {
    let mut sql = String::from(
        "DROP TABLE effect_log; DROP TABLE effect_days; ALTER TABLE actions DROP COLUMN closed; ALTER TABLE stack_actions DROP COLUMN closed; DROP TABLE retention; DROP TABLE pruned_legacy; ALTER TABLE actions DROP COLUMN legacy; ALTER TABLE stack_actions DROP COLUMN legacy;",
    );
    if to == 5 {
        sql += "DROP INDEX actions_digest; DROP INDEX stack_actions_digest; ALTER TABLE actions DROP COLUMN expires_at; ALTER TABLE stack_actions DROP COLUMN expires_at;";
    }
    sql += &format!("PRAGMA user_version={to};");
    store.conn.execute_batch(&sql).unwrap();
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
    downgrade(store, 5);
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
    // Schema 5 never pruned anything.
    assert_eq!(watermark_of(&store), i64::MIN);
}

#[test]
fn a_pruned_pre_upgrade_receipt_is_no_longer_held_even_above_its_back_filled_expiry() {
    let dir = private_dir();
    let store = ReceiptStore::open(dir.path()).unwrap();
    for i in 0..RETAINED_ACTIONS + 1 {
        insert(
            &store,
            "actions",
            &id('a', i),
            &format!("da{i}"),
            &format!("container:a{i}"),
            "verified",
            0,
        );
    }
    settled_stack(&store, 0, 0);
    downgrade(store, 5);
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    assert_eq!(
        count(&store, "SELECT count(*) FROM actions WHERE legacy=0"),
        0
    );
    assert_eq!(
        count(&store, "SELECT count(*) FROM stack_actions WHERE legacy=1"),
        1
    );
    let back_filled: i64 = store
        .conn
        .query_row("SELECT max(expires_at) FROM actions", [], |r| r.get(0))
        .unwrap();
    // Rows written after the upgrade are not legacy.
    insert(
        &store,
        "actions",
        &id('b', 0),
        "db0",
        "container:b0",
        "verified",
        back_filled + 1,
    );
    assert_eq!(
        count(&store, "SELECT count(*) FROM actions WHERE legacy=0"),
        1
    );
    // Pruned once its back-filled expiry is an hour past.
    let removed = prune_all(&mut store, back_filled + RETAIN_AFTER_EXPIRY);
    assert_eq!(removed, 2);
    assert_eq!(watermark_of(&store), back_filled);
    // Its plan's real expiry is unknown; the back-fill assumed the clock never
    // stepped back. Asked with any expiry, even far above the watermark, the
    // pruned receipt is "no longer held", never "never recorded".
    let pruned = count(&store, "SELECT count(*) FROM pruned_legacy");
    assert_eq!(pruned, 2);
    let gone: String = store
        .conn
        .query_row(
            "SELECT action FROM pruned_legacy WHERE action LIKE 'a%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        absence(&store, ResolveKind::Container, &gone, back_filled + 100_000),
        ReceiptAbsence::NoLongerHeld
    );
    assert_eq!(
        absence(
            &store,
            ResolveKind::Container,
            &id('c', 0),
            back_filled + 100_000
        ),
        ReceiptAbsence::NeverRecorded
    );
    // The tombstones survive a restart.
    drop(store);
    let store = ReceiptStore::open(dir.path()).unwrap();
    assert_eq!(
        absence(&store, ResolveKind::Container, &gone, back_filled + 100_000),
        ReceiptAbsence::NoLongerHeld
    );
}

#[test]
fn a_schema_six_store_that_may_have_pruned_without_a_watermark_never_claims_never_recorded() {
    let dir = private_dir();
    let store = ReceiptStore::open(dir.path()).unwrap();
    insert(
        &store,
        "actions",
        &id('a', 0),
        "da0",
        "container:a0",
        "verified",
        1_000,
    );
    downgrade(store, 6);
    let store = ReceiptStore::open(dir.path()).unwrap();
    assert_eq!(watermark_of(&store), i64::MAX);
    assert_eq!(
        count(&store, "SELECT count(*) FROM actions WHERE legacy=1"),
        1
    );
    assert_eq!(
        absence(&store, ResolveKind::Container, &id('b', 0), i64::MAX - 1),
        ReceiptAbsence::NoLongerHeld
    );
}

/// 64 lower-case hex digits, as action ids, digests and container ids are,
/// spread like random ones (splitmix64), so index updates land on scattered
/// pages rather than one end of each B-tree.
fn hex64(seed: u64) -> String {
    let mut state = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    (0..4)
        .map(|_| {
            state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            format!("{:016x}", z ^ (z >> 31))
        })
        .collect()
}

/// A receipt store at its retained capacity and past it: `RETAINED_ACTIONS`
/// plus a batch of standalone receipts and `RETAINED_STACKS` plus four
/// stacks of `MAX_STACK_MEMBERS` members, every row settled, expired and
/// legacy (so pruning also writes a tombstone for each), with random ids and
/// receipts the size of real ones: about 450 bytes a container receipt and
/// 6.5 KiB a full stack receipt (16 members' snapshots and states).
fn full_receipt_store() -> (tempfile::TempDir, ReceiptStore) {
    let dir = private_dir();
    let store = ReceiptStore::open(dir.path()).unwrap();
    let member = format!("{{\"receipt\":\"{}\"}}", "m".repeat(430));
    let stack = format!("{{\"receipt\":\"{}\"}}", "s".repeat(6600));
    let mut seed = 0u64;
    let mut next = || {
        seed += 1;
        hex64(seed)
    };
    let tx = store.conn.unchecked_transaction().unwrap();
    for _ in 0..RETAINED_ACTIONS + PRUNE_BATCH {
        tx.execute(
            "INSERT INTO actions(action,digest,resource,state,receipt,operation,expires_at,legacy) VALUES(?,?,?,'verified',?,'restart',0,1)",
            params![next(), next(), format!("container:{}", next()), member],
        )
        .unwrap();
    }
    for _ in 0..RETAINED_STACKS + 4 {
        let digest = next();
        tx.execute(
            "INSERT INTO stack_actions(action,digest,resource,state,receipt,operation,expires_at,legacy) VALUES(?,?,?,'verified',?,'restart',0,1)",
            params![next(), digest, format!("stack:{}", &next()[..12]), stack],
        )
        .unwrap();
        for _ in 0..limeos_domain::MAX_STACK_MEMBERS {
            tx.execute(
                "INSERT INTO actions(action,digest,resource,state,receipt,operation,expires_at,legacy) VALUES(?,?,?,'verified',?,'restart',0,1)",
                params![next(), digest, format!("container:{}", next()), member],
            )
            .unwrap();
        }
    }
    tx.commit().unwrap();
    (dir, store)
}

/// The receipt store's largest transaction: a stack's first write, which
/// prunes a full batch and then inserts the stack row and its sixteen member
/// rows (`stack::write_stack`), at `cache_kib`. Returns the frames committed,
/// the WAL bytes on disk before the commit (a spill; see the persistence
/// crate's `measured`) and the rows pruned.
fn largest_transaction(
    store: &mut ReceiptStore,
    directory: &std::path::Path,
    cache_kib: u32,
) -> (u64, u64, u32) {
    let wal = directory.join("receipts.sqlite-wal");
    store
        .conn
        .execute_batch(&format!(
            "PRAGMA wal_checkpoint(TRUNCATE); PRAGMA wal_autocheckpoint=0; PRAGMA cache_size=-{cache_kib}; PRAGMA shrink_memory;"
        ))
        .unwrap();
    assert_eq!(std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0), 0);
    let member = format!("{{\"receipt\":\"{}\"}}", "m".repeat(430));
    let stack = format!("{{\"receipt\":\"{}\"}}", "s".repeat(6600));
    let tx = store.conn.transaction().unwrap();
    let removed = prune(&tx, 100_000).unwrap();
    let digest = hex64(1 << 40);
    tx.execute(
        "INSERT INTO stack_actions(action,digest,resource,state,receipt,operation,expires_at) VALUES(?,?,'stack:new','prepared',?,'restart',200000)",
        params![hex64((1 << 40) + 1), digest, stack],
    )
    .unwrap();
    for i in 0..limeos_domain::MAX_STACK_MEMBERS as u64 {
        tx.execute(
            "INSERT INTO actions(action,digest,resource,state,receipt,operation,expires_at) VALUES(?,?,?,'prepared',?,'restart',200000)",
            params![hex64((1 << 41) + i), digest, format!("container:{}", hex64((1 << 42) + i)), member],
        )
        .unwrap();
    }
    let spilled = std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0);
    tx.commit().unwrap();
    let total = std::fs::metadata(&wal).unwrap().len();
    ((total - 32) / (4096 + 24), spilled, removed)
}

#[test]
#[ignore = "measurement: run with --ignored --nocapture"]
fn measure_the_largest_receipt_transaction_at_each_cache_size() {
    for kib in [128u32, 256, 384, 512, 768, 2000] {
        let (dir, mut store) = full_receipt_store();
        let (pages, spilled, removed) = largest_transaction(&mut store, dir.path(), kib);
        eprintln!(
            "receipt store, prune and stack insert: {kib} KiB cache: {removed} pruned, {pages} pages, {spilled} bytes spilled"
        );
    }
}

#[test]
#[ignore = "measurement: run with --ignored --nocapture"]
fn measure_the_open_reads_of_a_full_store() {
    let (dir, store) = full_receipt_store();
    store
        .conn
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    drop(store);
    let io = || -> (u64, u64) {
        let text = std::fs::read_to_string("/proc/thread-self/io").unwrap();
        let field = |name: &str| -> u64 {
            text.lines()
                .find_map(|l| l.strip_prefix(name)?.strip_prefix(": "))
                .unwrap()
                .trim()
                .parse()
                .unwrap()
        };
        (field("syscr"), field("rchar"))
    };
    let file = std::fs::metadata(dir.path().join("receipts.sqlite"))
        .unwrap()
        .len();
    for _ in 0..3 {
        let before = io();
        let opened = ReceiptStore::open(dir.path()).map(drop);
        let after = io();
        assert!(opened.is_ok());
        eprintln!(
            "open at {} KiB cache: {} read calls, {} KiB read, file {} KiB",
            crate::CACHE_KIB,
            after.0 - before.0,
            (after.1 - before.1) / 1024,
            file / 1024
        );
    }
}

/// The store's largest transaction (a stack's first write: a full prune
/// batch, tombstones for legacy rows, then the stack and sixteen member rows,
/// with random ids and real receipt sizes) fits its cache without spilling.
/// Every other transaction the store runs writes one receipt or less than a
/// full batch, so this one covers them; none is left out.
#[test]
fn the_largest_receipt_transaction_does_not_spill() {
    let (dir, mut store) = full_receipt_store();
    let (pages, spilled, removed) = largest_transaction(&mut store, dir.path(), crate::CACHE_KIB);
    assert_eq!(removed, PRUNE_BATCH);
    assert_eq!(spilled, 0, "{pages} pages spilled {spilled} bytes");
    eprintln!(
        "prune and stack insert: {pages} pages; cache {} pages",
        crate::CACHE_KIB / 4
    );
}

#[test]
fn the_receipt_connection_has_its_cache_size() {
    let dir = private_dir();
    let store = ReceiptStore::open(dir.path()).unwrap();
    let cache: i64 = store
        .conn
        .pragma_query_value(None, "cache_size", |r| r.get(0))
        .unwrap();
    assert_eq!(cache, -(crate::CACHE_KIB as i64));
    drop(store);
    // Reopened: no migration runs, the size is the same.
    let store = ReceiptStore::open(dir.path()).unwrap();
    let cache: i64 = store
        .conn
        .pragma_query_value(None, "cache_size", |r| r.get(0))
        .unwrap();
    assert_eq!(cache, -(crate::CACHE_KIB as i64));
    // The store is this crate's only SQLite connection outside its tests; a
    // new one fails here until it has a cache size too.
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut sites = Vec::new();
    let mut stack = vec![src.clone()];
    while let Some(directory) = stack.pop() {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let name = path
                .strip_prefix(&src)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if name.ends_with("tests.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let text = text.split("#[cfg(test)]\nmod tests").next().unwrap();
            sites.extend(text.matches("Connection::open").map(|_| name.clone()));
        }
    }
    assert_eq!(
        sites,
        ["lib.rs"],
        "a new SQLite connection: give it a cache size and list it here"
    );
}
