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
/// The plan a request carries for `digest`: the fixture plan with that digest,
/// or, for a digest no fixture plan has, one of `kind` that does not hash to
/// it, which the executor refuses as `InvalidInput`.
fn plan_for(kind: ResolveKind, digest: &str) -> ActionPlan {
    let known = [
        ActionPlan::Container(single('d', 'a').plan),
        ActionPlan::Container(single('d', 'b').plan),
        ActionPlan::Stack(stack_request().plan),
    ];
    let named = known.iter().find(|plan| {
        let body = match plan {
            ActionPlan::Container(plan) => encode(plan).unwrap(),
            ActionPlan::Stack(plan) => encode(plan).unwrap(),
        };
        limeos_identity::digest(&body) == digest
    });
    named
        .or_else(|| known.iter().find(|plan| plan.kind() == kind))
        .unwrap()
        .clone()
}
async fn outcome(
    store: &mut ReceiptStore,
    engine: &RecordingEngine,
    kind: ResolveKind,
    action: &str,
    digest: &str,
    evidence: &ResolutionEvidence,
    declared: DeclaredOutcome,
) -> Result<ResolveOutcome> {
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
            &plan_for(kind, digest),
        )
        .await
}
/// As `outcome`, for a call that must settle a row or fail.
async fn resolve(
    store: &mut ReceiptStore,
    engine: &RecordingEngine,
    kind: ResolveKind,
    action: &str,
    digest: &str,
    evidence: &ResolutionEvidence,
    declared: DeclaredOutcome,
) -> Result<ResolveReceipt> {
    match outcome(store, engine, kind, action, digest, evidence, declared).await? {
        ResolveOutcome::Resolved(receipt) => Ok(receipt),
        other => panic!("not settled: {other:?}"),
    }
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
            &plan_for(kind, &first.plan_digest),
        )
        .await;
    assert_eq!(wrong.unwrap_err().0, ErrorCode::Conflict);
    // An action the store never recorded has nothing to free.
    assert_eq!(
        outcome(
            &mut store,
            &engine,
            kind,
            &"1".repeat(64),
            &first.plan_digest,
            &evidence,
            DeclaredOutcome::VerifiedDone
        )
        .await
        .unwrap(),
        ResolveOutcome::Absent(limeos_domain::ReceiptAbsence::NeverRecorded)
    );
    // A plan digest the carried plan does not hash to is an invalid request.
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
        ErrorCode::InvalidInput
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
                &plan_for(kind, &first.plan_digest),
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
            &plan_for(kind, &first.plan_digest),
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
            &plan_for(kind, &first.plan_digest),
        )
        .await;
    assert_eq!(other_admin.unwrap_err().0, ErrorCode::Conflict);
}

/// A receipt the executor finished while core still holds the job.
struct Terminal {
    _dir: tempfile::TempDir,
    store: ReceiptStore,
    engine: RecordingEngine,
    kind: ResolveKind,
    action: String,
    digest: String,
    targets: Vec<ContainerSnapshot>,
}
impl Terminal {
    /// The stored row: state and receipt body.
    fn row(&self) -> (String, String) {
        let table = match self.kind {
            ResolveKind::Container => "actions",
            ResolveKind::Stack => "stack_actions",
        };
        self.store
            .conn
            .query_row(
                &format!("SELECT state,receipt FROM {table} WHERE action=?"),
                [&self.action],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    }
    /// Resolve on what the administrator is shown now; also returns how many
    /// inspections the executor itself made.
    async fn resolve(&mut self, declared: DeclaredOutcome) -> (Result<ResolveOutcome>, usize) {
        let evidence = shown(&self.engine, &self.targets).await;
        let inspections = self.engine.inspections.load(Ordering::SeqCst);
        let result = outcome(
            &mut self.store,
            &self.engine,
            self.kind,
            &self.action,
            &self.digest,
            &evidence,
            declared,
        )
        .await;
        (
            result,
            self.engine.inspections.load(Ordering::SeqCst) - inspections,
        )
    }
}
/// A container receipt left `verified` or `precondition_changed`, or a stack
/// receipt left `verified`, `precondition_changed` or `partly_done`.
async fn terminal(kind: ResolveKind, recorded: ExecutionState) -> Terminal {
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let engine = RecordingEngine::new(false);
    if kind == ResolveKind::Container {
        let request = single('d', 'a');
        if recorded == ExecutionState::Verified {
            store
                .execute(&engine, &ceiling(), 1001, &request, || 201)
                .await
                .unwrap();
            let after = engine.members.lock().unwrap()[0].clone();
            let receipt = store
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
            assert_eq!(receipt.state, recorded);
        } else {
            // The plan expired between the Prepared row and the Engine call.
            let calls = AtomicUsize::new(0);
            let receipt = store
                .execute(&engine, &ceiling(), 1001, &request, || {
                    if calls.fetch_add(1, Ordering::SeqCst) < 3 {
                        201
                    } else {
                        600
                    }
                })
                .await
                .unwrap();
            assert_eq!(receipt.state, recorded);
        }
        return Terminal {
            _dir: dir,
            store,
            engine,
            kind,
            action: request.action,
            digest: request.plan_digest,
            targets: vec![snap('a', true)],
        };
    }
    let request = stack_request();
    let targets: Vec<_> = request.plan.targets().into_iter().cloned().collect();
    let states = match recorded {
        ExecutionState::Verified => [ExecutionState::Verified; 2],
        ExecutionState::PreconditionChanged => [ExecutionState::PreconditionChanged; 2],
        _ => [
            ExecutionState::Verified,
            ExecutionState::PreconditionChanged,
        ],
    };
    for (target, state) in targets.iter().zip(states) {
        if state == ExecutionState::Verified {
            engine.restarted(target.resource.chars().last().unwrap());
        }
    }
    let members: Vec<_> = targets
        .iter()
        .zip(states)
        .map(|(t, state)| limeos_domain::StackMemberReceipt {
            resource: t.resource.clone(),
            state,
            error: None,
        })
        .collect();
    let receipt = limeos_domain::StackReceipt {
        action: request.action.clone(),
        plan_digest: request.plan_digest.clone(),
        operation: request.plan.operation,
        before: request.plan.expected.clone(),
        state: limeos_domain::aggregate_state(&members),
        members,
        error: None,
    };
    assert_eq!(receipt.state, recorded);
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
    Terminal {
        _dir: dir,
        store,
        engine,
        kind,
        action: request.action,
        digest: request.plan_digest,
        targets,
    }
}

#[tokio::test]
async fn a_terminal_receipt_is_resolvable_only_by_a_declaration_its_outcome_allows() {
    use DeclaredOutcome::*;
    use ExecutionState::*;
    let table: [(ResolveKind, ExecutionState, &[DeclaredOutcome]); 5] = [
        (
            ResolveKind::Container,
            Verified,
            &[VerifiedDone, AcceptedDivergence],
        ),
        (
            ResolveKind::Container,
            PreconditionChanged,
            &[NotDone, AcceptedDivergence],
        ),
        (
            ResolveKind::Stack,
            Verified,
            &[VerifiedDone, AcceptedDivergence],
        ),
        (
            ResolveKind::Stack,
            PreconditionChanged,
            &[NotDone, AcceptedDivergence],
        ),
        (ResolveKind::Stack, PartlyDone, &[AcceptedDivergence]),
    ];
    for (kind, recorded, allowed) in table {
        for declared in [VerifiedDone, NotDone, AcceptedDivergence] {
            let case = format!("{kind:?} {recorded:?} {declared:?}");
            let mut t = terminal(kind, recorded).await;
            let (mutations, before) = (t.engine.mutations(), t.row());
            let (result, inspected) = t.resolve(declared).await;
            let result = result.unwrap();
            assert_eq!(t.engine.mutations(), mutations, "{case}");
            if !allowed.contains(&declared) {
                // Refused with the reason, before any Engine contact; nothing changed.
                assert_eq!(
                    result,
                    ResolveOutcome::Refused(limeos_domain::ResolveRefusal {
                        recorded,
                        declared,
                        allowed: allowed.to_vec(),
                    }),
                    "{case}"
                );
                assert_eq!(t.row(), before, "{case}");
                assert_eq!(inspected, 0, "{case}");
                assert!(
                    t.store.resolution(kind, &t.action).unwrap().is_none(),
                    "{case}"
                );
                continue;
            }
            let ResolveOutcome::Resolved(confirmed) = result else {
                panic!("{case}: {result:?}");
            };
            // Resolved, with the original receipt kept as recorded.
            assert_eq!(t.row(), (RESOLVED.to_owned(), before.1.clone()), "{case}");
            assert_eq!(
                t.store.resolution(kind, &t.action).unwrap(),
                Some(confirmed.clone()),
                "{case}"
            );
            match kind {
                ResolveKind::Container => assert_eq!(
                    t.store
                        .receipt(&t.action, &t.digest)
                        .unwrap()
                        .unwrap()
                        .state,
                    recorded,
                    "{case}"
                ),
                ResolveKind::Stack => assert_eq!(
                    t.store
                        .stack_receipt(&t.action, &t.digest)
                        .unwrap()
                        .unwrap()
                        .state,
                    recorded,
                    "{case}"
                ),
            }
            // An idempotent retry is answered from the row, without inspecting.
            let (again, inspected) = t.resolve(declared).await;
            assert_eq!(
                again.unwrap(),
                ResolveOutcome::Resolved(confirmed),
                "{case}"
            );
            assert_eq!(inspected, 0, "{case}");
            // A different declaration cannot rewrite it.
            let other = if declared == AcceptedDivergence {
                allowed[0]
            } else {
                AcceptedDivergence
            };
            if other != declared {
                assert_eq!(
                    t.resolve(other).await.0.unwrap_err().0,
                    ErrorCode::Conflict,
                    "{case}"
                );
            }
        }
    }
}

#[tokio::test]
async fn a_terminal_receipt_still_needs_fresh_matching_evidence_and_the_peer_ceiling() {
    let mut t = terminal(ResolveKind::Container, ExecutionState::Verified).await;
    let evidence = shown(&t.engine, &t.targets).await;
    // The container changed after the administrator looked.
    t.engine.restarted('a');
    assert_eq!(
        outcome(
            &mut t.store,
            &t.engine,
            t.kind,
            &t.action,
            &t.digest,
            &evidence,
            DeclaredOutcome::VerifiedDone
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::Conflict
    );
    let other = ContainerCeiling {
        managed_containers: Vec::new(),
        ..ceiling()
    };
    for (uid, ceiling) in [(1002, ceiling()), (1001, other)] {
        let result = t
            .store
            .resolve_action(
                &t.engine,
                &ceiling,
                uid,
                t.kind,
                &t.action,
                &t.digest,
                &evidence.digest().unwrap(),
                DeclaredOutcome::VerifiedDone,
                &by(),
                &plan_for(t.kind, &t.digest),
            )
            .await;
        assert_eq!(result.unwrap_err().0, ErrorCode::Forbidden);
    }
    assert_eq!(t.row().0, "verified");
}

#[tokio::test]
async fn kind_and_table_must_agree() {
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let engine = RecordingEngine::new(false);
    let first = single('d', 'a');
    store
        .execute(&engine, &ceiling(), 1001, &first, || 201)
        .await
        .unwrap();
    let evidence = shown(&engine, &[snap('a', true)]).await;
    // A container plan cannot be resolved as a stack action.
    assert_eq!(
        outcome(
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
        ErrorCode::InvalidInput
    );
    // A stack plan names the stack table, which holds no row for this action.
    let stack = stack_request();
    assert_eq!(
        outcome(
            &mut store,
            &engine,
            ResolveKind::Stack,
            &first.action,
            &stack.plan_digest,
            &evidence,
            DeclaredOutcome::AcceptedDivergence
        )
        .await
        .unwrap(),
        ResolveOutcome::Absent(limeos_domain::ReceiptAbsence::NeverRecorded)
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
        conn.execute_batch("DROP TABLE retention; DROP TABLE pruned_legacy; DROP INDEX actions_resource_lock; DROP INDEX stack_actions_lock; DROP TABLE actions; DROP TABLE stack_actions; CREATE TABLE actions(action TEXT PRIMARY KEY,digest TEXT NOT NULL,resource TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('prepared','effect_accepted','verified','precondition_changed','outcome_unknown')),receipt TEXT NOT NULL,operation TEXT NOT NULL DEFAULT 'restart' CHECK(operation IN ('restart','start','stop'))); CREATE UNIQUE INDEX actions_resource_lock ON actions(resource) WHERE state IN ('prepared','effect_accepted','outcome_unknown'); CREATE TABLE stack_actions(action TEXT PRIMARY KEY,digest TEXT NOT NULL,resource TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('prepared','effect_accepted','verified','precondition_changed','outcome_unknown','partly_done')),receipt TEXT NOT NULL,operation TEXT NOT NULL CHECK(operation IN ('restart','start','stop'))); CREATE UNIQUE INDEX stack_actions_lock ON stack_actions(resource) WHERE state IN ('prepared','effect_accepted','outcome_unknown'); INSERT INTO actions VALUES('a1','d','container:r','outcome_unknown','{}','restart'); PRAGMA user_version=4;").unwrap();
    }
    let store = ReceiptStore::open(dir.path()).unwrap();
    let version: u32 = store
        .conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, crate::RECEIPT_SCHEMA_VERSION);
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
