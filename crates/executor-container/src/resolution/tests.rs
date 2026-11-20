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
/// Records every mutating Engine call. Effects change the listed state; a
/// failing effect still happens, as a lost response would.
struct RecordingEngine {
    members: Mutex<Vec<ContainerSnapshot>>,
    mutations: AtomicUsize,
    inspections: AtomicUsize,
    fail: bool,
}
impl RecordingEngine {
    fn new(fail: bool) -> Self {
        Self {
            members: Mutex::new(vec![snap('a', true), snap('b', true), snap('c', false)]),
            mutations: AtomicUsize::new(0),
            inspections: AtomicUsize::new(0),
            fail,
        }
    }
    fn mutations(&self) -> usize {
        self.mutations.load(Ordering::SeqCst)
    }
    fn effect(&self, id: &str) -> Result<()> {
        self.mutations.fetch_add(1, Ordering::SeqCst);
        let mut members = self.members.lock().unwrap();
        let member = members
            .iter_mut()
            .find(|m| m.resource == format!("container:{id}"))
            .unwrap();
        member.started_at = "2026-10-05T07:05:00.000000000Z".into();
        member.running = true;
        if self.fail {
            Err(Error(ErrorCode::Unavailable))
        } else {
            Ok(())
        }
    }
    fn restarted(&self, c: char) {
        let mut members = self.members.lock().unwrap();
        let m = members
            .iter_mut()
            .find(|m| m.resource.contains(&c.to_string().repeat(64)))
            .unwrap();
        m.started_at = "2026-10-05T07:09:00.000000000Z".into();
    }
    fn gone(&self, c: char) {
        self.members
            .lock()
            .unwrap()
            .retain(|m| !m.resource.contains(&c.to_string().repeat(64)));
    }
}
impl Engine for RecordingEngine {
    async fn inspect(&self, id: &str) -> Result<ContainerSnapshot> {
        self.inspections.fetch_add(1, Ordering::SeqCst);
        self.members
            .lock()
            .unwrap()
            .iter()
            .find(|m| m.resource == format!("container:{id}"))
            .cloned()
            .ok_or(Error(ErrorCode::NotFound))
    }
    async fn restart(&self, id: &str) -> Result<()> {
        self.effect(id)
    }
    async fn start(&self, id: &str) -> Result<()> {
        self.effect(id)
    }
    async fn stop(&self, id: &str) -> Result<()> {
        self.effect(id)
    }
    async fn project_members(&self, _: &str) -> Result<Vec<ContainerSnapshot>> {
        Ok(self.members.lock().unwrap().clone())
    }
}
fn single(action: char, c: char) -> RestartRequest {
    single_from(action, snap(c, true))
}
fn single_from(action: char, expected: ContainerSnapshot) -> RestartRequest {
    let plan = RestartPlan {
        operation: ContainerAction::Restart,
        id: "c".repeat(64),
        version: 1,
        principal: "alice".into(),
        grant_revision: 1,
        expected,
        created_at: 200,
        expires_at: 500,
    };
    RestartRequest {
        action: action.to_string().repeat(64),
        plan_digest: limeos_identity::digest(&encode(&plan).unwrap()),
        plan,
    }
}
fn stack_request() -> StackRequest {
    let plan = StackPlan {
        id: "c".repeat(64),
        version: limeos_domain::STACK_VERSION,
        operation: ContainerAction::Restart,
        principal: "alice".into(),
        grant_revision: 1,
        catalog_stack: "registered".into(),
        expected: StackSnapshot::new(
            "media",
            vec![snap('a', true), snap('b', true), snap('c', false)],
        )
        .unwrap(),
        created_at: 200,
        expires_at: 500,
    };
    StackRequest {
        action: "d".repeat(64),
        plan_digest: limeos_identity::digest(&encode(&plan).unwrap()),
        plan,
    }
}
fn by() -> String {
    "9".repeat(64)
}
/// What an administrator was shown: the fresh measurement of these targets.
async fn shown(engine: &RecordingEngine, targets: &[ContainerSnapshot]) -> ResolutionEvidence {
    let mut current = Vec::new();
    for t in targets {
        current.push(
            engine
                .inspect(t.resource.strip_prefix("container:").unwrap())
                .await
                .ok(),
        );
    }
    ResolutionEvidence::observe(
        ContainerAction::Restart,
        &targets.iter().collect::<Vec<_>>(),
        &current,
    )
    .unwrap()
}
fn count(store: &ReceiptStore, sql: &str) -> u32 {
    store.conn.query_row(sql, [], |r| r.get(0)).unwrap()
}
async fn resolve(
    store: &mut ReceiptStore,
    engine: &RecordingEngine,
    kind: ResolveKind,
    action: &str,
    digest: &str,
    evidence: &ResolutionEvidence,
    declared: DeclaredOutcome,
) -> Result<ResolveReceipt> {
    store
        .resolve_action(
            engine,
            &ceiling(),
            1001,
            kind,
            action,
            digest,
            &evidence.digest().unwrap(),
            declared,
            &by(),
        )
        .await
}

#[tokio::test]
async fn resolved_container_action_frees_the_index_keeps_the_receipt_and_sends_nothing() {
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let engine = RecordingEngine::new(true);
    let first = single('d', 'a');
    let receipt = store
        .execute(&engine, &ceiling(), 1001, &first, || 201)
        .await
        .unwrap();
    assert_eq!(receipt.state, ExecutionState::OutcomeUnknown);
    // While unresolved the resource is held.
    // Planned against the container as it is now, so only the lock can refuse it.
    let next = single_from('e', engine.members.lock().unwrap()[0].clone());
    assert_eq!(
        store
            .execute(&engine, &ceiling(), 1001, &next, || 201)
            .await
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    let mutations = engine.mutations();
    let evidence = shown(&engine, &[snap('a', true)]).await;
    let confirmed = resolve(
        &mut store,
        &engine,
        ResolveKind::Container,
        &first.action,
        &first.plan_digest,
        &evidence,
        DeclaredOutcome::VerifiedDone,
    )
    .await
    .unwrap();
    assert_eq!(confirmed.action, first.action);
    assert_eq!(confirmed.resolved_by, by());
    // Zero Engine mutations: only read-only inspections were made.
    assert_eq!(engine.mutations(), mutations);
    let (state, stored): (String, String) = store
        .conn
        .query_row(
            "SELECT state,receipt FROM actions WHERE action=?",
            [&first.action],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, "resolved");
    assert_eq!(stored, encode(&receipt).unwrap());
    assert_eq!(
        store.receipt(&first.action, &first.plan_digest).unwrap(),
        Some(receipt)
    );
    assert_eq!(
        store
            .resolution(ResolveKind::Container, &first.action)
            .unwrap(),
        Some(confirmed)
    );
    // The index freed: a new action on the same container may now prepare.
    store
        .execute(&engine, &ceiling(), 1001, &next, || 201)
        .await
        .unwrap();
    assert_eq!(engine.mutations(), mutations + 1);
    // A resolved action can no longer be verified, and a restart is durable.
    assert_eq!(
        store
            .verify(
                &engine,
                &ceiling(),
                1001,
                &first.action,
                &first.plan_digest,
                &snap('a', true)
            )
            .await
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    drop(store);
    let store = ReceiptStore::open(dir.path()).unwrap();
    assert!(
        store
            .resolution(ResolveKind::Container, &first.action)
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn stale_evidence_wrong_ids_and_unsupported_declarations_are_refused_and_change_nothing() {
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let engine = RecordingEngine::new(true);
    let first = single('d', 'a');
    store
        .execute(&engine, &ceiling(), 1001, &first, || 201)
        .await
        .unwrap();
    let mutations = engine.mutations();
    let evidence = shown(&engine, &[snap('a', true)]).await;
    let kind = ResolveKind::Container;
    // The restart did change the container, so "not done" is unsupported.
    assert_eq!(
        resolve(
            &mut store,
            &engine,
            kind,
            &first.action,
            &first.plan_digest,
            &evidence,
            DeclaredOutcome::NotDone
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::Conflict
    );
    // Wrong digest, unknown action, wrong plan digest.
    let wrong = store
        .resolve_action(
            &engine,
            &ceiling(),
            1001,
            kind,
            &first.action,
            &first.plan_digest,
            &"0".repeat(64),
            DeclaredOutcome::AcceptedDivergence,
            &by(),
        )
        .await;
    assert_eq!(wrong.unwrap_err().0, ErrorCode::Conflict);
    assert_eq!(
        resolve(
            &mut store,
            &engine,
            kind,
            &"1".repeat(64),
            &first.plan_digest,
            &evidence,
            DeclaredOutcome::VerifiedDone
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::NotFound
    );
    assert_eq!(
        resolve(
            &mut store,
            &engine,
            kind,
            &first.action,
            &"1".repeat(64),
            &evidence,
            DeclaredOutcome::VerifiedDone
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::Conflict
    );
    // The container changed after the administrator looked: stale evidence.
    engine.restarted('a');
    assert_eq!(
        resolve(
            &mut store,
            &engine,
            kind,
            &first.action,
            &first.plan_digest,
            &evidence,
            DeclaredOutcome::VerifiedDone
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::Conflict
    );
    // A vanished container is stale evidence too.
    engine.gone('a');
    assert_eq!(
        resolve(
            &mut store,
            &engine,
            kind,
            &first.action,
            &first.plan_digest,
            &evidence,
            DeclaredOutcome::AcceptedDivergence
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::Conflict
    );
    // Authority is the ceiling's, not the caller's word.
    let other = ContainerCeiling {
        managed_containers: Vec::new(),
        ..ceiling()
    };
    for (uid, ceiling) in [(1002, ceiling()), (1001, other)] {
        let result = store
            .resolve_action(
                &engine,
                &ceiling,
                uid,
                kind,
                &first.action,
                &first.plan_digest,
                &evidence.digest().unwrap(),
                DeclaredOutcome::VerifiedDone,
                &by(),
            )
            .await;
        assert_eq!(result.unwrap_err().0, ErrorCode::Forbidden);
    }
    let bad = store
        .resolve_action(
            &engine,
            &ceiling(),
            1001,
            kind,
            "../x",
            &first.plan_digest,
            &evidence.digest().unwrap(),
            DeclaredOutcome::VerifiedDone,
            &by(),
        )
        .await;
    assert_eq!(bad.unwrap_err().0, ErrorCode::InvalidInput);
    // Nothing resolved, nothing sent, the resource is still held.
    assert_eq!(engine.mutations(), mutations);
    assert_eq!(
        count(
            &store,
            "SELECT count(*) FROM actions WHERE state='resolved'"
        ),
        0
    );
    assert!(store.resolution(kind, &first.action).unwrap().is_none());
}

#[tokio::test]
async fn repeating_a_resolution_is_idempotent_and_a_different_one_is_refused() {
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let engine = RecordingEngine::new(true);
    let first = single('d', 'a');
    store
        .execute(&engine, &ceiling(), 1001, &first, || 201)
        .await
        .unwrap();
    let evidence = shown(&engine, &[snap('a', true)]).await;
    let kind = ResolveKind::Container;
    let once = resolve(
        &mut store,
        &engine,
        kind,
        &first.action,
        &first.plan_digest,
        &evidence,
        DeclaredOutcome::VerifiedDone,
    )
    .await
    .unwrap();
    // The container changes afterwards: a repeat is answered from the row, not
    // re-measured, so a core that lost the response still gets its confirmation.
    engine.restarted('a');
    let inspections = engine.inspections.load(Ordering::SeqCst);
    let again = resolve(
        &mut store,
        &engine,
        kind,
        &first.action,
        &first.plan_digest,
        &evidence,
        DeclaredOutcome::VerifiedDone,
    )
    .await
    .unwrap();
    assert_eq!(once, again);
    assert_eq!(engine.inspections.load(Ordering::SeqCst), inspections);
    // A different declaration, evidence or administrator cannot rewrite it.
    assert_eq!(
        resolve(
            &mut store,
            &engine,
            kind,
            &first.action,
            &first.plan_digest,
            &evidence,
            DeclaredOutcome::AcceptedDivergence
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::Conflict
    );
    let other_admin = store
        .resolve_action(
            &engine,
            &ceiling(),
            1001,
            kind,
            &first.action,
            &first.plan_digest,
            &evidence.digest().unwrap(),
            DeclaredOutcome::VerifiedDone,
            &"8".repeat(64),
        )
        .await;
    assert_eq!(other_admin.unwrap_err().0, ErrorCode::Conflict);
}

#[tokio::test]
async fn an_action_that_no_longer_holds_the_resource_is_not_resolvable() {
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let engine = RecordingEngine::new(false);
    let first = single('d', 'a');
    store
        .execute(&engine, &ceiling(), 1001, &first, || 201)
        .await
        .unwrap();
    let after = engine.members.lock().unwrap()[0].clone();
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
    let evidence = shown(&engine, &[snap('a', true)]).await;
    assert_eq!(
        resolve(
            &mut store,
            &engine,
            ResolveKind::Container,
            &first.action,
            &first.plan_digest,
            &evidence,
            DeclaredOutcome::AcceptedDivergence
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::Conflict
    );
    // Kind and table must agree: a container action is not a stack action.
    assert_eq!(
        resolve(
            &mut store,
            &engine,
            ResolveKind::Stack,
            &first.action,
            &first.plan_digest,
            &evidence,
            DeclaredOutcome::AcceptedDivergence
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::NotFound
    );
}

#[tokio::test]
async fn a_resolved_stack_frees_the_stack_and_every_active_member_row() {
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let engine = RecordingEngine::new(true);
    let request = stack_request();
    let receipt = store
        .execute_stack(&engine, &ceiling(), 1001, &request, || 201)
        .await
        .unwrap();
    assert_eq!(receipt.state, ExecutionState::OutcomeUnknown);
    let mutations = engine.mutations();
    let targets: Vec<_> = request.plan.targets().into_iter().cloned().collect();
    let evidence = shown(&engine, &targets).await;
    assert_eq!(evidence.targets.len(), 2);
    let kind = ResolveKind::Stack;
    // Held: a second row for a locked member violates the unique index.
    let duplicate = "INSERT INTO actions(action,digest,resource,state,receipt,operation) VALUES('h','d','container:".to_string() + &"a".repeat(64) + "','prepared','{}','restart')";
    assert!(store.conn.execute(&duplicate, []).is_err());
    assert_eq!(
        resolve(
            &mut store,
            &engine,
            kind,
            &request.action,
            &request.plan_digest,
            &evidence,
            DeclaredOutcome::NotDone
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::Conflict
    );
    let confirmed = resolve(
        &mut store,
        &engine,
        kind,
        &request.action,
        &request.plan_digest,
        &evidence,
        DeclaredOutcome::AcceptedDivergence,
    )
    .await
    .unwrap();
    assert_eq!(engine.mutations(), mutations);
    assert_eq!(
        count(
            &store,
            "SELECT count(*) FROM stack_actions WHERE state='resolved'"
        ),
        1
    );
    assert_eq!(
        count(
            &store,
            "SELECT count(*) FROM actions WHERE state='resolved'"
        ),
        1
    ); // fail-stop left the second member terminal (never sent), so it keeps its state
    assert_eq!(
        store.resolution(kind, &request.action).unwrap(),
        Some(confirmed.clone())
    );
    // Lookups still work and return the receipt as recorded; recovery does not
    // rewrite or re-verify a resolved stack.
    assert_eq!(
        store
            .reconcile_stack(&request.action, &request.plan_digest)
            .unwrap()
            .unwrap(),
        receipt
    );
    assert_eq!(
        store
            .verify_stack(
                &engine,
                &ceiling(),
                1001,
                &request.action,
                &request.plan_digest,
                &evidence_snapshots(&engine)
            )
            .await
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    // Idempotent repeat.
    assert_eq!(
        resolve(
            &mut store,
            &engine,
            kind,
            &request.action,
            &request.plan_digest,
            &evidence,
            DeclaredOutcome::AcceptedDivergence
        )
        .await
        .unwrap(),
        confirmed
    );
    // The stack and its members are free for a new stack action.
    let mut again = stack_request();
    again.action = "7".repeat(64);
    let engine_ok = RecordingEngine::new(false);
    store
        .execute_stack(&engine_ok, &ceiling(), 1001, &again, || 201)
        .await
        .unwrap();
}
fn evidence_snapshots(engine: &RecordingEngine) -> Vec<ContainerSnapshot> {
    engine.members.lock().unwrap().clone()
}

#[tokio::test]
async fn a_prepared_stack_left_by_a_crash_can_be_resolved_and_a_vanished_member_is_divergence() {
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let engine = RecordingEngine::new(false);
    let request = stack_request();
    // Simulate a crash after the Prepared transaction and before any effect.
    let receipt = limeos_domain::StackReceipt {
        action: request.action.clone(),
        plan_digest: request.plan_digest.clone(),
        operation: request.plan.operation,
        before: request.plan.expected.clone(),
        members: request
            .plan
            .targets()
            .iter()
            .map(|t| limeos_domain::StackMemberReceipt {
                resource: t.resource.clone(),
                state: ExecutionState::Prepared,
                error: None,
            })
            .collect(),
        state: ExecutionState::Prepared,
        error: None,
    };
    store
        .write_stack(
            &receipt,
            &[],
            Some(crate::stack::Insert {
                expires_at: request.plan.expires_at,
                now: 201,
            }),
        )
        .unwrap();
    engine.gone('b');
    let targets: Vec<_> = request.plan.targets().into_iter().cloned().collect();
    let evidence = shown(&engine, &targets).await;
    assert!(evidence.targets.iter().any(|t| t.is_absent()));
    let kind = ResolveKind::Stack;
    for declared in [DeclaredOutcome::NotDone, DeclaredOutcome::VerifiedDone] {
        assert_eq!(
            resolve(
                &mut store,
                &engine,
                kind,
                &request.action,
                &request.plan_digest,
                &evidence,
                declared
            )
            .await
            .unwrap_err()
            .0,
            ErrorCode::Conflict
        );
    }
    resolve(
        &mut store,
        &engine,
        kind,
        &request.action,
        &request.plan_digest,
        &evidence,
        DeclaredOutcome::AcceptedDivergence,
    )
    .await
    .unwrap();
    assert_eq!(engine.mutations(), 0);
    assert_eq!(
        count(
            &store,
            "SELECT count(*) FROM actions WHERE state='resolved'"
        ),
        2
    );
}

#[test]
fn resolved_stays_out_of_both_unique_index_lists_and_inside_both_state_checks() {
    let dir = private_dir();
    let store = ReceiptStore::open(dir.path()).unwrap();
    for (index, table) in [
        ("actions_resource_lock", "actions"),
        ("stack_actions_lock", "stack_actions"),
    ] {
        let sql: String = store
            .conn
            .query_row("SELECT sql FROM sqlite_schema WHERE name=?", [index], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(
            sql.contains("'prepared'") && !sql.contains("resolved"),
            "{sql}"
        );
        let ddl: String = store
            .conn
            .query_row("SELECT sql FROM sqlite_schema WHERE name=?", [table], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(ddl.contains("'resolved'"));
    }
    // Unknown states are still refused by the CHECK.
    assert!(
        store
            .conn
            .execute(
                "INSERT INTO actions(action,digest,resource,state,receipt,operation) VALUES('x','y','container:z','bogus','{}','restart')",
                [],
            )
            .is_err()
    );
}

#[test]
fn schema_four_upgrade_keeps_rows_and_adds_the_resolved_state() {
    let dir = private_dir();
    drop(ReceiptStore::open(dir.path()).unwrap());
    {
        let conn = Connection::open(dir.path().join("receipts.sqlite")).unwrap();
        conn.execute_batch("DROP INDEX actions_resource_lock; DROP INDEX stack_actions_lock; DROP TABLE actions; DROP TABLE stack_actions; CREATE TABLE actions(action TEXT PRIMARY KEY,digest TEXT NOT NULL,resource TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('prepared','effect_accepted','verified','precondition_changed','outcome_unknown')),receipt TEXT NOT NULL,operation TEXT NOT NULL DEFAULT 'restart' CHECK(operation IN ('restart','start','stop'))); CREATE UNIQUE INDEX actions_resource_lock ON actions(resource) WHERE state IN ('prepared','effect_accepted','outcome_unknown'); CREATE TABLE stack_actions(action TEXT PRIMARY KEY,digest TEXT NOT NULL,resource TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('prepared','effect_accepted','verified','precondition_changed','outcome_unknown','partly_done')),receipt TEXT NOT NULL,operation TEXT NOT NULL CHECK(operation IN ('restart','start','stop'))); CREATE UNIQUE INDEX stack_actions_lock ON stack_actions(resource) WHERE state IN ('prepared','effect_accepted','outcome_unknown'); INSERT INTO actions VALUES('a1','d','container:r','outcome_unknown','{}','restart'); PRAGMA user_version=4;").unwrap();
    }
    let store = ReceiptStore::open(dir.path()).unwrap();
    let version: u32 = store
        .conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 6);
    assert_eq!(
        count(&store, "SELECT count(*) FROM actions WHERE action='a1'"),
        1
    );
    // The preserved row still holds its resource through the new index.
    assert!(
        store
            .conn
            .execute(
                "INSERT INTO actions(action,digest,resource,state,receipt,operation) VALUES('a2','d','container:r','prepared','{}','restart')",
                [],
            )
            .is_err()
    );
    store
        .conn
        .execute(
            "UPDATE actions SET state='resolved',resolution='{}' WHERE action='a1'",
            [],
        )
        .unwrap();
    store
        .conn
        .execute(
            "INSERT INTO actions(action,digest,resource,state,receipt,operation) VALUES('a2','d','container:r','prepared','{}','restart')",
            [],
        )
        .unwrap();
}
