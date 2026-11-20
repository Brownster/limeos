use super::*;
use crate::retention::{RETAIN_AFTER_EXPIRY, RETAINED_ACTIONS, RETAINED_STACKS, prune};
use limeos_domain::{DeclaredOutcome, StackSnapshot};
use std::sync::Mutex;

/// Plans in these tests are created at `CREATED` and expire at `EXPIRES`.
const CREATED: i64 = 1000;
const EXPIRES: i64 = CREATED + limeos_domain::PLAN_TTL_SECONDS;
/// Core asks after the plan has expired by its clock...
const ASKED: i64 = EXPIRES + 60;
/// ...and the executor's clock then steps back to before the expiry.
const STEPPED_BACK: i64 = CREATED + 1;

fn private_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}
fn snap(c: char, running: bool) -> ContainerSnapshot {
    ContainerSnapshot {
        resource: format!("container:{}", c.to_string().repeat(64)),
        image: format!("sha256:{}", "b".repeat(64)),
        started_at: "2026-10-05T07:00:00.000000000Z".into(),
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
fn stack_request(action: &str) -> StackRequest {
    let plan = StackPlan {
        id: "c".repeat(64),
        version: limeos_domain::STACK_VERSION,
        operation: ContainerAction::Restart,
        principal: "alice".into(),
        grant_revision: 1,
        catalog_stack: "registered".into(),
        expected: StackSnapshot::new("media", vec![snap('a', true), snap('b', true)]).unwrap(),
        created_at: CREATED,
        expires_at: EXPIRES,
    };
    StackRequest {
        action: action.into(),
        plan_digest: limeos_identity::digest(&encode(&plan).unwrap()),
        plan,
    }
}
fn container_request(action: &str, expires_at: i64) -> RestartRequest {
    container_request_on('a', action, expires_at)
}
fn container_request_on(target: char, action: &str, expires_at: i64) -> RestartRequest {
    let plan = ContainerPlan {
        operation: ContainerAction::Restart,
        id: "c".repeat(64),
        version: 1,
        principal: "alice".into(),
        grant_revision: 1,
        expected: snap(target, true),
        created_at: expires_at - limeos_domain::PLAN_TTL_SECONDS,
        expires_at,
    };
    RestartRequest {
        action: action.into(),
        plan_digest: limeos_identity::digest(&encode(&plan).unwrap()),
        plan,
    }
}
/// Records every Engine call, reads included; the state never changes.
#[derive(Default)]
struct Recorder {
    calls: Mutex<Vec<String>>,
}
impl Recorder {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
    fn record(&self, call: &str) {
        self.calls.lock().unwrap().push(call.into());
    }
}
impl Engine for Recorder {
    async fn inspect(&self, id: &str) -> Result<ContainerSnapshot> {
        self.record("inspect");
        [snap('a', true), snap('b', true), snap('c', true)]
            .into_iter()
            .find(|m| m.resource == format!("container:{id}"))
            .ok_or(Error(ErrorCode::NotFound))
    }
    async fn restart(&self, _: &str) -> Result<()> {
        self.record("restart");
        Ok(())
    }
    async fn project_members(&self, _: &str) -> Result<Vec<ContainerSnapshot>> {
        self.record("project_members");
        Ok(vec![snap('a', true), snap('b', true)])
    }
}
fn rows(store: &ReceiptStore, table: &str) -> u32 {
    store
        .conn
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}
fn close_stack(store: &mut ReceiptStore, request: &StackRequest, now: i64) -> Lookup<StackReceipt> {
    store
        .lookup_stack(
            &ceiling(),
            1001,
            &request.action,
            &request.plan_digest,
            &request.plan,
            true,
            now,
        )
        .unwrap()
}
fn close(store: &mut ReceiptStore, request: &RestartRequest, now: i64) -> Lookup<RestartReceipt> {
    store
        .lookup(
            &ceiling(),
            1001,
            &request.action,
            &request.plan_digest,
            &request.plan,
            true,
            now,
        )
        .unwrap()
}
fn raise_watermark(store: &ReceiptStore, to: i64) {
    store
        .conn
        .execute("UPDATE retention SET watermark=?1 WHERE id=1", [to])
        .unwrap();
}

#[tokio::test]
async fn a_closed_stack_action_refuses_a_late_execute_whatever_the_clock_says() {
    let dir = private_dir();
    let mut store = ReceiptStore::open_at(dir.path(), crate::TEST_STORE_CREATED).unwrap();
    let request = stack_request(&"d".repeat(64));
    let Lookup::Receipt { receipt, closed } = close_stack(&mut store, &request, ASKED) else {
        panic!("a never-recorded action is answered with its closure receipt");
    };
    assert!(closed);
    assert_eq!(receipt.state, ExecutionState::PreconditionChanged);
    assert!(
        receipt.members.iter().all(
            |m| m.state == ExecutionState::PreconditionChanged && m.error == Some(CLOSED_ERROR)
        )
    );
    // The stack row and one member row per target, none holding a lock.
    assert_eq!(
        (rows(&store, "stack_actions"), rows(&store, "actions")),
        (1, 2)
    );
    // A delayed execute request with unchanged preconditions, arriving while
    // the executor's clock has stepped back to before the plan's expiry.
    let engine = Recorder::default();
    for clock in [STEPPED_BACK, EXPIRES - 1, ASKED] {
        let late = store
            .execute_stack(&engine, &ceiling(), 1001, &request, || clock)
            .await
            .unwrap();
        assert_eq!(late, receipt, "{clock}");
    }
    assert!(engine.calls().is_empty());
    // A reopened store keeps the action taken.
    drop(store);
    let mut store = ReceiptStore::open_at(dir.path(), crate::TEST_STORE_CREATED).unwrap();
    let late = store
        .execute_stack(&engine, &ceiling(), 1001, &request, || STEPPED_BACK)
        .await
        .unwrap();
    assert_eq!(late, receipt);
    assert!(engine.calls().is_empty());
}

#[tokio::test]
async fn a_closed_container_action_refuses_a_late_execute_whatever_the_clock_says() {
    let dir = private_dir();
    let mut store = ReceiptStore::open_at(dir.path(), crate::TEST_STORE_CREATED).unwrap();
    let request = container_request(&"d".repeat(64), EXPIRES);
    let Lookup::Receipt { receipt, closed } = close(&mut store, &request, ASKED) else {
        panic!("a never-recorded action is answered with its closure receipt");
    };
    assert!(closed);
    assert_eq!(receipt.state, ExecutionState::PreconditionChanged);
    assert_eq!(receipt.error, Some(CLOSED_ERROR));
    assert_eq!(receipt.before, request.plan.expected);
    let engine = Recorder::default();
    for clock in [STEPPED_BACK, EXPIRES - 1] {
        let late = store
            .execute(&engine, &ceiling(), 1001, &request, || clock)
            .await
            .unwrap();
        assert_eq!(late, receipt);
    }
    assert!(engine.calls().is_empty());
    // A closure holds no lock: another action on the same container runs.
    let other = container_request(&"e".repeat(64), EXPIRES);
    let ran = store
        .execute(&engine, &ceiling(), 1001, &other, || STEPPED_BACK)
        .await
        .unwrap();
    assert_eq!(ran.state, ExecutionState::EffectAccepted);
    assert_eq!(engine.calls(), ["inspect", "restart"]);
}

#[tokio::test]
async fn a_status_read_without_the_close_intent_records_nothing() {
    let dir = private_dir();
    let mut store = ReceiptStore::open_at(dir.path(), crate::TEST_STORE_CREATED).unwrap();
    let stack = stack_request(&"d".repeat(64));
    let single = container_request(&"e".repeat(64), EXPIRES);
    for _ in 0..2 {
        assert_eq!(
            store
                .lookup_stack(
                    &ceiling(),
                    1001,
                    &stack.action,
                    &stack.plan_digest,
                    &stack.plan,
                    false,
                    ASKED
                )
                .unwrap(),
            Lookup::Absent {
                absence: ReceiptAbsence::NeverRecorded,
                closed: false
            }
        );
        assert_eq!(
            store
                .lookup(
                    &ceiling(),
                    1001,
                    &single.action,
                    &single.plan_digest,
                    &single.plan,
                    false,
                    ASKED
                )
                .unwrap(),
            Lookup::Absent {
                absence: ReceiptAbsence::NeverRecorded,
                closed: false
            }
        );
    }
    assert_eq!(
        (rows(&store, "stack_actions"), rows(&store, "actions")),
        (0, 0)
    );
    // Nothing was taken: the execute still runs.
    let engine = Recorder::default();
    let ran = store
        .execute_stack(&engine, &ceiling(), 1001, &stack, || STEPPED_BACK)
        .await
        .unwrap();
    assert_eq!(ran.state, ExecutionState::EffectAccepted);
}

#[tokio::test]
async fn an_execute_that_prepared_first_is_reported_and_a_closure_first_is_returned() {
    // Execute first: the lookup reports its receipt and closes nothing.
    let dir = private_dir();
    let mut store = ReceiptStore::open_at(dir.path(), crate::TEST_STORE_CREATED).unwrap();
    let engine = Recorder::default();
    let stack = stack_request(&"d".repeat(64));
    let ran = store
        .execute_stack(&engine, &ceiling(), 1001, &stack, || STEPPED_BACK)
        .await
        .unwrap();
    assert_eq!(
        close_stack(&mut store, &stack, ASKED),
        Lookup::Receipt {
            receipt: ran,
            closed: false
        }
    );
    assert_eq!(
        store.closure(ResolveKind::Stack, &stack.action).unwrap(),
        None
    );
    // A container action whose Prepared row is all there is (the executor
    // died before the Engine answered): reported as it stands, not closed.
    let single = container_request_on('c', &"e".repeat(64), EXPIRES);
    let prepared = RestartReceipt {
        action: single.action.clone(),
        plan_digest: single.plan_digest.clone(),
        operation: ContainerAction::Restart,
        before: single.plan.expected.clone(),
        state: ExecutionState::Prepared,
        error: None,
    };
    store
        .prepare(&prepared, single.plan.expires_at, STEPPED_BACK)
        .unwrap();
    assert_eq!(
        close(&mut store, &single, ASKED),
        Lookup::Receipt {
            receipt: prepared,
            closed: false
        }
    );
    assert_eq!(
        store
            .closure(ResolveKind::Container, &single.action)
            .unwrap(),
        None
    );
    let calls = engine.calls().len();
    // Lookup first: the execute returns the closure and reaches no Engine.
    let late = stack_request(&"f".repeat(64));
    let Lookup::Receipt { receipt, .. } = close_stack(&mut store, &late, ASKED) else {
        panic!("closure receipt expected");
    };
    assert_eq!(
        store
            .execute_stack(&engine, &ceiling(), 1001, &late, || STEPPED_BACK)
            .await
            .unwrap(),
        receipt
    );
    assert_eq!(engine.calls().len(), calls);
}

#[tokio::test]
async fn a_no_longer_held_action_is_closed_but_still_answered_as_no_longer_held() {
    let dir = private_dir();
    let mut store = ReceiptStore::open_at(dir.path(), crate::TEST_STORE_CREATED).unwrap();
    // A receipt for a plan expiring this late may have been pruned.
    raise_watermark(&store, EXPIRES);
    let stack = stack_request(&"d".repeat(64));
    let single = container_request(&"e".repeat(64), EXPIRES);
    fn answer<R>() -> Lookup<R> {
        Lookup::Absent {
            absence: ReceiptAbsence::NoLongerHeld,
            closed: true,
        }
    }
    for _ in 0..2 {
        assert_eq!(close_stack(&mut store, &stack, ASKED), answer());
        assert_eq!(close(&mut store, &single, ASKED), answer());
    }
    // A plain read now sees the closure too.
    assert_eq!(
        store
            .lookup(
                &ceiling(),
                1001,
                &single.action,
                &single.plan_digest,
                &single.plan,
                false,
                ASKED
            )
            .unwrap(),
        answer()
    );
    assert_eq!(
        store.closure(ResolveKind::Stack, &stack.action).unwrap(),
        Some(ReceiptAbsence::NoLongerHeld)
    );
    // A late execute is refused, and never told nothing was sent.
    let engine = Recorder::default();
    assert_eq!(
        store
            .execute_stack(&engine, &ceiling(), 1001, &stack, || STEPPED_BACK)
            .await
            .unwrap_err()
            .0,
        ErrorCode::Expired
    );
    assert_eq!(
        store
            .execute(&engine, &ceiling(), 1001, &single, || STEPPED_BACK)
            .await
            .unwrap_err()
            .0,
        ErrorCode::Expired
    );
    assert!(engine.calls().is_empty());
}

async fn resolve_stack(
    store: &mut ReceiptStore,
    engine: &Recorder,
    stack: &StackRequest,
) -> ResolveOutcome {
    store
        .resolve_action(
            engine,
            &ceiling(),
            1001,
            ResolveKind::Stack,
            &stack.action,
            &stack.plan_digest,
            &"e".repeat(64),
            DeclaredOutcome::NotDone,
            &"a".repeat(64),
            &ActionPlan::Stack(stack.plan.clone()),
            false,
            ASKED,
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn resolving_an_absent_action_closes_it_and_a_closure_row_is_answered_as_absent() {
    let dir = private_dir();
    let mut store = ReceiptStore::open_at(dir.path(), crate::TEST_STORE_CREATED).unwrap();
    let engine = Recorder::default();
    let stack = stack_request(&"d".repeat(64));
    for _ in 0..2 {
        assert_eq!(
            resolve_stack(&mut store, &engine, &stack).await,
            ResolveOutcome::Absent(ReceiptAbsence::NeverRecorded)
        );
    }
    assert_eq!(
        store.closure(ResolveKind::Stack, &stack.action).unwrap(),
        Some(ReceiptAbsence::NeverRecorded)
    );
    let late = store
        .execute_stack(&engine, &ceiling(), 1001, &stack, || STEPPED_BACK)
        .await
        .unwrap();
    assert_eq!(late.state, ExecutionState::PreconditionChanged);
    assert!(engine.calls().is_empty());
}

#[allow(clippy::too_many_arguments)]
async fn resolve_declaring(
    store: &mut ReceiptStore,
    engine: &Recorder,
    kind: ResolveKind,
    action: &str,
    digest: &str,
    plan: ActionPlan,
    declared: DeclaredOutcome,
) -> ResolveOutcome {
    resolve_as(store, engine, kind, action, digest, plan, declared, false).await
}

/// As `resolve_declaring`, saying whether core disputes the action.
#[allow(clippy::too_many_arguments)]
async fn resolve_as(
    store: &mut ReceiptStore,
    engine: &Recorder,
    kind: ResolveKind,
    action: &str,
    digest: &str,
    plan: ActionPlan,
    declared: DeclaredOutcome,
    disputed: bool,
) -> ResolveOutcome {
    store
        .resolve_action(
            engine,
            &ceiling(),
            1001,
            kind,
            action,
            digest,
            &"e".repeat(64),
            declared,
            &"a".repeat(64),
            &plan,
            disputed,
            ASKED,
        )
        .await
        .unwrap()
}

/// A never-recorded closure proves nothing was ever sent, as a refusal an
/// execute wrote does: it allows only the declarations a
/// `precondition_changed` receipt allows, whether the resolution finds the
/// closure or writes it.
#[tokio::test]
async fn a_never_recorded_closure_refuses_a_verified_done_declaration() {
    use limeos_domain::ResolveRefusal;
    let dir = private_dir();
    let mut store = ReceiptStore::open_at(dir.path(), crate::TEST_STORE_CREATED).unwrap();
    let engine = Recorder::default();
    let stack = stack_request(&"d".repeat(64));
    let single = container_request(&"e".repeat(64), EXPIRES);
    let cases = [
        (
            ResolveKind::Stack,
            stack.action.clone(),
            stack.plan_digest.clone(),
            ActionPlan::Stack(stack.plan.clone()),
        ),
        (
            ResolveKind::Container,
            single.action.clone(),
            single.plan_digest.clone(),
            ActionPlan::Container(single.plan.clone()),
        ),
    ];
    let refusal = ResolveOutcome::Refused(ResolveRefusal {
        recorded: ExecutionState::PreconditionChanged,
        declared: DeclaredOutcome::VerifiedDone,
        allowed: vec![
            DeclaredOutcome::NotDone,
            DeclaredOutcome::AcceptedDivergence,
        ],
    });
    for (kind, action, digest, plan) in &cases {
        // The first ask closes the action; the closure refuses every repeat.
        for _ in 0..2 {
            assert_eq!(
                resolve_declaring(
                    &mut store,
                    &engine,
                    *kind,
                    action,
                    digest,
                    plan.clone(),
                    DeclaredOutcome::VerifiedDone
                )
                .await,
                refusal,
                "{kind:?}"
            );
        }
        assert_eq!(
            store.closure(*kind, action).unwrap(),
            Some(ReceiptAbsence::NeverRecorded)
        );
        for declared in [
            DeclaredOutcome::NotDone,
            DeclaredOutcome::AcceptedDivergence,
        ] {
            assert_eq!(
                resolve_declaring(
                    &mut store,
                    &engine,
                    *kind,
                    action,
                    digest,
                    plan.clone(),
                    declared
                )
                .await,
                ResolveOutcome::Absent(ReceiptAbsence::NeverRecorded),
                "{kind:?} {declared:?}"
            );
        }
    }
    assert!(engine.calls().is_empty());
}

/// Core disputes the action (the store changed while it held the job, or it
/// holds a result the closure contradicts): a never-recorded closure is then
/// not evidence, so every declaration is left to the operator's own
/// measurement, whether the resolution writes the closure or finds it. The
/// same closure without the dispute still refuses "verified done".
#[tokio::test]
async fn a_disputed_never_recorded_closure_leaves_every_declaration_to_the_operator() {
    let dir = private_dir();
    let mut store = ReceiptStore::open_at(dir.path(), crate::TEST_STORE_CREATED).unwrap();
    let engine = Recorder::default();
    let stack = stack_request(&"d".repeat(64));
    let single = container_request(&"e".repeat(64), EXPIRES);
    let cases = [
        (
            ResolveKind::Stack,
            stack.action.clone(),
            stack.plan_digest.clone(),
            ActionPlan::Stack(stack.plan.clone()),
        ),
        (
            ResolveKind::Container,
            single.action.clone(),
            single.plan_digest.clone(),
            ActionPlan::Container(single.plan.clone()),
        ),
    ];
    for (kind, action, digest, plan) in &cases {
        // Written by the first ask, found by the rest.
        for declared in [
            DeclaredOutcome::VerifiedDone,
            DeclaredOutcome::VerifiedDone,
            DeclaredOutcome::NotDone,
            DeclaredOutcome::AcceptedDivergence,
        ] {
            assert_eq!(
                resolve_as(
                    &mut store,
                    &engine,
                    *kind,
                    action,
                    digest,
                    plan.clone(),
                    declared,
                    true
                )
                .await,
                ResolveOutcome::Absent(ReceiptAbsence::NeverRecorded),
                "{kind:?} {declared:?}"
            );
        }
        assert!(matches!(
            resolve_declaring(
                &mut store,
                &engine,
                *kind,
                action,
                digest,
                plan.clone(),
                DeclaredOutcome::VerifiedDone
            )
            .await,
            ResolveOutcome::Refused(_)
        ));
    }
    assert!(engine.calls().is_empty());
}

/// A no_longer_held closure claims nothing about what ran: every
/// declaration stays the operator's judgement.
#[tokio::test]
async fn a_no_longer_held_closure_leaves_every_declaration_to_the_operator() {
    let dir = private_dir();
    let mut store = ReceiptStore::open_at(dir.path(), crate::TEST_STORE_CREATED).unwrap();
    raise_watermark(&store, EXPIRES);
    let engine = Recorder::default();
    let stack = stack_request(&"d".repeat(64));
    for declared in [
        DeclaredOutcome::VerifiedDone,
        DeclaredOutcome::NotDone,
        DeclaredOutcome::AcceptedDivergence,
    ] {
        assert_eq!(
            resolve_declaring(
                &mut store,
                &engine,
                ResolveKind::Stack,
                &stack.action,
                &stack.plan_digest,
                ActionPlan::Stack(stack.plan.clone()),
                declared
            )
            .await,
            ResolveOutcome::Absent(ReceiptAbsence::NoLongerHeld),
            "{declared:?}"
        );
    }
}

#[test]
fn closure_rows_are_pruned_like_settled_history_and_never_while_the_plan_is_valid() {
    let dir = private_dir();
    let mut store = ReceiptStore::open_at(dir.path(), crate::TEST_STORE_CREATED).unwrap();
    // A full standalone budget whose plans stay valid throughout.
    for i in 0..RETAINED_ACTIONS {
        store
            .conn
            .execute(
                "INSERT INTO actions(action,digest,resource,state,receipt,operation,expires_at) VALUES(?1,?1,?1,'verified','{}','restart',?2)",
                params![format!("filler{i}"), i64::MAX / 2],
            )
            .unwrap();
    }
    let first = container_request(&"d".repeat(64), EXPIRES);
    close(&mut store, &first, ASKED);
    // Over budget, and every later insert prunes; the closure stays as long
    // as its plan, expired for less than the retention period, could matter.
    for (i, now) in [ASKED, EXPIRES + RETAIN_AFTER_EXPIRY - 1]
        .into_iter()
        .enumerate()
    {
        let next = container_request(&format!("{:064x}", i + 1), now + 100);
        close(&mut store, &next, now);
        assert!(
            store
                .closure(ResolveKind::Container, &first.action)
                .unwrap()
                .is_some(),
            "{now}"
        );
    }
    // Once it has been expired for the retention period it is settled history.
    let next = container_request(&"e".repeat(64), EXPIRES + RETAIN_AFTER_EXPIRY + 100);
    close(&mut store, &next, EXPIRES + RETAIN_AFTER_EXPIRY);
    assert!(
        store
            .receipt(&first.action, &first.plan_digest)
            .unwrap()
            .is_none()
    );
    // Counted by the watermark like any pruned row: the absence is now
    // `no_longer_held`, and asking again closes it as such.
    assert!(crate::retention::watermark(&store.conn).unwrap() >= EXPIRES);
    assert_eq!(
        close(&mut store, &first, EXPIRES + RETAIN_AFTER_EXPIRY),
        Lookup::Absent {
            absence: ReceiptAbsence::NoLongerHeld,
            closed: true
        }
    );
}

#[test]
fn a_stack_closure_is_pruned_whole_with_its_member_rows() {
    let dir = private_dir();
    let mut store = ReceiptStore::open_at(dir.path(), crate::TEST_STORE_CREATED).unwrap();
    for i in 0..RETAINED_STACKS {
        store
            .conn
            .execute(
                "INSERT INTO stack_actions(action,digest,resource,state,receipt,operation,expires_at) VALUES(?1,?1,'stack:media','verified','{}','restart',?2)",
                params![format!("filler{i}"), i64::MAX / 2],
            )
            .unwrap();
    }
    let stack = stack_request(&"d".repeat(64));
    close_stack(&mut store, &stack, ASKED);
    assert_eq!(rows(&store, "actions"), 2);
    let prune_at = |store: &mut ReceiptStore, now| {
        let tx = store.conn.transaction().unwrap();
        let removed = prune(&tx, now).unwrap();
        tx.commit().unwrap();
        removed
    };
    assert_eq!(prune_at(&mut store, EXPIRES + RETAIN_AFTER_EXPIRY - 1), 0);
    assert_eq!(prune_at(&mut store, EXPIRES + RETAIN_AFTER_EXPIRY), 3);
    assert_eq!(rows(&store, "actions"), 0);
    assert!(
        store
            .stack_receipt(&stack.action, &stack.plan_digest)
            .unwrap()
            .is_none()
    );
}

/// The case retention could not exclude before: a closure pruned once its
/// plan had been expired for the retention period, then a backward clock step
/// to before the plan's expiry, then the delayed execute request. It finds no
/// row and both expiry checks pass, but its plan expires at or below the
/// watermark, so it is refused before prepare, with no Engine call.
#[tokio::test]
async fn a_pruned_closure_refuses_a_delayed_execute_after_the_clock_steps_back() {
    let dir = private_dir();
    let mut store = ReceiptStore::open_at(dir.path(), crate::TEST_STORE_CREATED).unwrap();
    for i in 0..RETAINED_ACTIONS {
        store
            .conn
            .execute(
                "INSERT INTO actions(action,digest,resource,state,receipt,operation,expires_at) VALUES(?1,?1,?1,'verified','{}','restart',?2)",
                params![format!("filler{i}"), i64::MAX / 2],
            )
            .unwrap();
    }
    let first = container_request(&"d".repeat(64), EXPIRES);
    assert!(matches!(
        close(&mut store, &first, ASKED),
        Lookup::Receipt { closed: true, .. }
    ));
    let pruned_at = EXPIRES + RETAIN_AFTER_EXPIRY;
    close(
        &mut store,
        &container_request(&"e".repeat(64), pruned_at + 100),
        pruned_at,
    );
    assert!(
        store
            .receipt(&first.action, &first.plan_digest)
            .unwrap()
            .is_none()
    );
    let engine = Recorder::default();
    for clock in [STEPPED_BACK, EXPIRES - 1] {
        assert_eq!(
            store
                .execute(&engine, &ceiling(), 1001, &first, || clock)
                .await
                .unwrap_err()
                .0,
            ErrorCode::BeyondReceiptHistory
        );
    }
    assert!(engine.calls().is_empty());
    // Refused before prepare: nothing recorded, no lock taken.
    assert!(
        store
            .receipt(&first.action, &first.plan_digest)
            .unwrap()
            .is_none()
    );
    // A plan approved after the pruning still executes.
    let fresh = container_request(&"f".repeat(64), pruned_at + 400);
    let ran = store
        .execute(&engine, &ceiling(), 1001, &fresh, || pruned_at + 200)
        .await
        .unwrap();
    assert_eq!(ran.state, ExecutionState::EffectAccepted);
    assert_eq!(engine.calls(), ["inspect", "restart"]);
}

#[tokio::test]
async fn a_pruned_stack_closure_refuses_a_delayed_execute_after_the_clock_steps_back() {
    let dir = private_dir();
    let mut store = ReceiptStore::open_at(dir.path(), crate::TEST_STORE_CREATED).unwrap();
    for i in 0..RETAINED_STACKS {
        store
            .conn
            .execute(
                "INSERT INTO stack_actions(action,digest,resource,state,receipt,operation,expires_at) VALUES(?1,?1,'stack:media','verified','{}','restart',?2)",
                params![format!("filler{i}"), i64::MAX / 2],
            )
            .unwrap();
    }
    let stack = stack_request(&"d".repeat(64));
    close_stack(&mut store, &stack, ASKED);
    let tx = store.conn.transaction().unwrap();
    assert_eq!(prune(&tx, EXPIRES + RETAIN_AFTER_EXPIRY).unwrap(), 3);
    tx.commit().unwrap();
    let engine = Recorder::default();
    for clock in [STEPPED_BACK, EXPIRES - 1] {
        assert_eq!(
            store
                .execute_stack(&engine, &ceiling(), 1001, &stack, || clock)
                .await
                .unwrap_err()
                .0,
            ErrorCode::BeyondReceiptHistory
        );
    }
    assert!(engine.calls().is_empty());
    assert_eq!(rows(&store, "actions"), 0);
}

/// The execute barrier draws the line where an absence does: a plan
/// expiring exactly at the watermark may share its expiry with a pruned
/// receipt and is refused; one a second later is unknown to every pruned
/// receipt and runs.
#[tokio::test]
async fn the_execute_barrier_uses_the_watermark_as_absence_does() {
    let dir = private_dir();
    let mut store = ReceiptStore::open_at(dir.path(), crate::TEST_STORE_CREATED).unwrap();
    raise_watermark(&store, EXPIRES);
    let engine = Recorder::default();
    let at = container_request(&"d".repeat(64), EXPIRES);
    assert_eq!(
        store
            .receipt_absence(
                &at.action,
                &at.plan_digest,
                &ActionPlan::Container(at.plan.clone())
            )
            .unwrap(),
        ReceiptAbsence::NoLongerHeld
    );
    assert_eq!(
        store
            .execute(&engine, &ceiling(), 1001, &at, || STEPPED_BACK)
            .await
            .unwrap_err()
            .0,
        ErrorCode::BeyondReceiptHistory
    );
    let mut stack = stack_request(&"e".repeat(64));
    assert_eq!(stack.plan.expires_at, EXPIRES);
    assert_eq!(
        store
            .execute_stack(&engine, &ceiling(), 1001, &stack, || STEPPED_BACK)
            .await
            .unwrap_err()
            .0,
        ErrorCode::BeyondReceiptHistory
    );
    assert!(engine.calls().is_empty());
    // On a container outside the stack, which this action then holds.
    let after = container_request_on('c', &"f".repeat(64), EXPIRES + 1);
    assert_eq!(
        store
            .execute(&engine, &ceiling(), 1001, &after, || STEPPED_BACK + 1)
            .await
            .unwrap()
            .state,
        ExecutionState::EffectAccepted
    );
    stack.plan.created_at += 1;
    stack.plan.expires_at += 1;
    stack.plan_digest = limeos_identity::digest(&encode(&stack.plan).unwrap());
    stack.action = "c".repeat(64);
    let ran = store
        .execute_stack(&engine, &ceiling(), 1001, &stack, || STEPPED_BACK + 1)
        .await
        .unwrap();
    assert!(
        ran.members
            .iter()
            .all(|m| m.state == ExecutionState::EffectAccepted)
    );
}

#[test]
fn closure_kill_helper() {
    let Some(directory) = std::env::var_os("LIMEOS_CLOSURE_KILL_FIXTURE") else {
        return;
    };
    let mut store =
        ReceiptStore::open_at(std::path::Path::new(&directory), crate::TEST_STORE_CREATED).unwrap();
    close_stack(&mut store, &stack_request(&"d".repeat(64)), ASKED);
    // Killed after the closure is durable and before any answer is sent.
    std::process::exit(86);
}

#[tokio::test]
async fn a_process_killed_between_closing_and_answering_leaves_the_closure_and_the_same_answer() {
    let dir = private_dir();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "closure::tests::closure_kill_helper",
            "--nocapture",
        ])
        .env("LIMEOS_CLOSURE_KILL_FIXTURE", dir.path())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(86));
    let mut store = ReceiptStore::open_at(dir.path(), crate::TEST_STORE_CREATED).unwrap();
    let request = stack_request(&"d".repeat(64));
    assert_eq!(
        store.closure(ResolveKind::Stack, &request.action).unwrap(),
        Some(ReceiptAbsence::NeverRecorded)
    );
    let stored = store
        .stack_receipt(&request.action, &request.plan_digest)
        .unwrap()
        .unwrap();
    // The retry gets the answer the lost one would have carried, and writes
    // nothing more.
    for _ in 0..2 {
        assert_eq!(
            close_stack(&mut store, &request, ASKED),
            Lookup::Receipt {
                receipt: stored.clone(),
                closed: true
            }
        );
    }
    assert_eq!(
        (rows(&store, "stack_actions"), rows(&store, "actions")),
        (1, 2)
    );
    let engine = Recorder::default();
    assert_eq!(
        store
            .execute_stack(&engine, &ceiling(), 1001, &request, || STEPPED_BACK)
            .await
            .unwrap(),
        stored
    );
    assert!(engine.calls().is_empty());
}

#[test]
fn schema_seven_upgrade_adds_the_closure_column_and_keeps_every_row() {
    let dir = private_dir();
    let store = ReceiptStore::open_at(dir.path(), crate::TEST_STORE_CREATED).unwrap();
    store
        .conn
        .execute(
            "INSERT INTO actions(action,digest,resource,state,receipt,operation,expires_at) VALUES('old','dold','container:old','verified','{}','restart',5)",
            [],
        )
        .unwrap();
    store
        .conn
        .execute_batch(
            "ALTER TABLE actions DROP COLUMN closed; ALTER TABLE stack_actions DROP COLUMN closed; PRAGMA user_version=7;",
        )
        .unwrap();
    drop(store);
    let store = ReceiptStore::open_at(dir.path(), crate::TEST_STORE_CREATED).unwrap();
    let version: u32 = store
        .conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, crate::RECEIPT_SCHEMA_VERSION);
    let closed: Option<String> = store
        .conn
        .query_row("SELECT closed FROM actions WHERE action='old'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(closed, None);
    // A closure marker belongs only on a refusal row.
    assert!(
        store
            .conn
            .execute(
                "UPDATE actions SET closed='never_recorded' WHERE action='old'",
                []
            )
            .is_err()
    );
}

/// Files of a closed store, by name, for copying a store aside and back.
fn store_files(dir: &std::path::Path) -> Vec<(std::ffi::OsString, Vec<u8>)> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("receipts.sqlite")
        })
        .map(|e| (e.file_name(), std::fs::read(e.path()).unwrap()))
        .collect()
}
fn remove_store_files(dir: &std::path::Path) {
    for (name, _) in store_files(dir) {
        std::fs::remove_file(dir.join(name)).unwrap();
    }
}

/// A store created at `created` cannot know what ran before it existed. Any
/// plan that could predate it (created no later than `created`, so expiring
/// no later than `created + PLAN_TTL_SECONDS`) is answered `no_longer_held`
/// and refused at execute; a plan created afterwards is never recorded and
/// runs.
#[tokio::test]
async fn a_new_store_claims_no_knowledge_of_plans_that_could_predate_it() {
    let created = 1_000_000;
    let dir = private_dir();
    let mut store = ReceiptStore::open_at(dir.path(), created).unwrap();
    let floor = created + limeos_domain::PLAN_TTL_SECONDS;
    assert_eq!(crate::retention::watermark(&store.conn).unwrap(), floor);
    let engine = Recorder::default();
    let older = container_request(&"d".repeat(64), floor);
    assert_eq!(
        close(&mut store, &older, created + 1),
        Lookup::Absent {
            absence: ReceiptAbsence::NoLongerHeld,
            closed: true
        }
    );
    let unknown = container_request(&"e".repeat(64), floor);
    assert_eq!(
        store
            .execute(&engine, &ceiling(), 1001, &unknown, || created + 1)
            .await
            .unwrap_err()
            .0,
        ErrorCode::BeyondReceiptHistory
    );
    assert!(engine.calls().is_empty());
    // A plan approved a second after the store was created is unknown to
    // every earlier store: closing it says so, and executing it runs.
    let newer = container_request(&"f".repeat(64), floor + 1);
    assert!(matches!(
        close(&mut store, &newer, created + 2),
        Lookup::Receipt { closed: true, .. }
    ));
    let fresh = container_request_on('b', &"0".repeat(64), floor + 1);
    assert_eq!(
        store
            .execute(&engine, &ceiling(), 1001, &fresh, || created + 2)
            .await
            .unwrap()
            .state,
        ExecutionState::EffectAccepted
    );
    // The floor is set once, at creation: reopening keeps it.
    drop(store);
    let store = ReceiptStore::open_at(dir.path(), created + 10_000).unwrap();
    assert_eq!(crate::retention::watermark(&store.conn).unwrap(), floor);
    // The executor's `open` takes the creation time from the system clock.
    let other = private_dir();
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let store = ReceiptStore::open(other.path()).unwrap();
    let floor = crate::retention::watermark(&store.conn).unwrap();
    assert!(
        (before + limeos_domain::PLAN_TTL_SECONDS..=before + limeos_domain::PLAN_TTL_SECONDS + 60)
            .contains(&floor)
    );
}

/// The receipt store is lost between an executed action and core's
/// recovery lookup, and a new one is created: it must not certify the action
/// as never recorded.
#[tokio::test]
async fn a_recreated_store_never_certifies_an_executed_action_as_never_recorded() {
    let ran_at = 1_000_000;
    let dir = private_dir();
    let mut store = ReceiptStore::open_at(dir.path(), ran_at - 1_000).unwrap();
    let engine = Recorder::default();
    let stack = stack_request(&"d".repeat(64));
    let request = container_request_on(
        'c',
        &"e".repeat(64),
        ran_at - 1 + limeos_domain::PLAN_TTL_SECONDS,
    );
    let mut stack_late = stack.clone();
    stack_late.plan.created_at = ran_at - 1;
    stack_late.plan.expires_at = ran_at - 1 + limeos_domain::PLAN_TTL_SECONDS;
    stack_late.plan_digest = limeos_identity::digest(&encode(&stack_late.plan).unwrap());
    store
        .execute(&engine, &ceiling(), 1001, &request, || ran_at)
        .await
        .unwrap();
    store
        .execute_stack(&engine, &ceiling(), 1001, &stack_late, || ran_at)
        .await
        .unwrap();
    let before = store.epoch().to_owned();
    drop(store);
    remove_store_files(dir.path());
    let mut store = ReceiptStore::open_at(dir.path(), ran_at + 5).unwrap();
    assert_ne!(store.epoch(), before);
    fn gone<R>() -> Lookup<R> {
        Lookup::Absent {
            absence: ReceiptAbsence::NoLongerHeld,
            closed: true,
        }
    }
    assert_eq!(close(&mut store, &request, ran_at + 6), gone());
    assert_eq!(close_stack(&mut store, &stack_late, ran_at + 6), gone());
}

/// Every open of a store has a new epoch. A floor cannot see a store
/// restored from an earlier copy (its watermark is the copy's), and an
/// epoch fixed at creation would be the copy's too; a new epoch per open
/// lets core tell that the store it talks to is not the one it dispatched
/// under.
#[tokio::test]
async fn every_open_has_a_new_epoch_so_a_restored_copy_is_told_apart() {
    let created = 1_000_000;
    let dir = private_dir();
    let store = ReceiptStore::open_at(dir.path(), created).unwrap();
    let first = store.epoch().to_owned();
    assert!(limeos_domain::opaque_id(&first));
    drop(store);
    let copy = store_files(dir.path());
    let mut store = ReceiptStore::open_at(dir.path(), created + 100).unwrap();
    let dispatched = store.epoch().to_owned();
    assert_ne!(dispatched, first);
    let engine = Recorder::default();
    let request = container_request(&"d".repeat(64), created + 400);
    store
        .execute(&engine, &ceiling(), 1001, &request, || created + 200)
        .await
        .unwrap();
    drop(store);
    remove_store_files(dir.path());
    for (name, bytes) in &copy {
        std::fs::write(dir.path().join(name), bytes).unwrap();
        std::fs::set_permissions(
            dir.path().join(name),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
    }
    let mut store = ReceiptStore::open_at(dir.path(), created + 300).unwrap();
    assert!(![first.as_str(), dispatched.as_str()].contains(&store.epoch()));
    // The restored copy has no row for the action and its watermark cannot
    // cover it: only the epoch shows core that this is not the store the
    // action was dispatched to.
    assert!(matches!(
        close(&mut store, &request, created + 301),
        Lookup::Receipt { closed: true, .. }
    ));
}
