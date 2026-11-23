use super::*;
use limeos_domain::{StackPlan, StackSnapshot};
use std::sync::Mutex;

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
fn members() -> Vec<ContainerSnapshot> {
    vec![snap('a', true), snap('b', true), snap('c', false)]
}
fn request_for(operation: ContainerAction, members: Vec<ContainerSnapshot>) -> StackRequest {
    let plan = StackPlan {
        id: "c".repeat(64),
        version: limeos_domain::STACK_VERSION,
        operation,
        principal: "alice".into(),
        grant_revision: 1,
        catalog_stack: "registered".into(),
        expected: StackSnapshot::new("media", members).unwrap(),
        created_at: 200,
        expires_at: 500,
    };
    StackRequest {
        action: "d".repeat(64),
        plan_digest: limeos_identity::digest(&encode(&plan).unwrap()),
        plan,
    }
}
fn single_restart(expected: &ContainerSnapshot) -> RestartRequest {
    let plan = limeos_domain::RestartPlan {
        operation: ContainerAction::Restart,
        id: "c".repeat(64),
        version: 1,
        principal: "alice".into(),
        grant_revision: 1,
        expected: expected.clone(),
        created_at: 200,
        expires_at: 500,
    };
    RestartRequest {
        action: "9".repeat(64),
        plan_digest: limeos_identity::digest(&encode(&plan).unwrap()),
        plan,
    }
}
fn request(operation: ContainerAction) -> StackRequest {
    request_for(operation, members())
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
/// A fake Engine whose effects change the listed state, as a real one would.
struct StackEngine {
    members: Mutex<Vec<ContainerSnapshot>>,
    calls: Mutex<Vec<String>>,
    fail: Option<char>,
}
impl StackEngine {
    fn new() -> Self {
        Self::with(members(), None)
    }
    fn with(members: Vec<ContainerSnapshot>, fail: Option<char>) -> Self {
        Self {
            members: Mutex::new(members),
            calls: Mutex::new(Vec::new()),
            fail,
        }
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
    fn effect(&self, operation: &str, id: &str) -> Result<()> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("{operation} {}", &id[..1]));
        let mut members = self.members.lock().unwrap();
        let member = members
            .iter_mut()
            .find(|m| m.resource == format!("container:{id}"))
            .unwrap();
        // The failing effect still happens: a lost response is not a refusal.
        match operation {
            "stop" => member.running = false,
            _ => {
                member.running = true;
                member.started_at = "2026-10-05T07:05:00Z".into();
            }
        }
        if self.fail.is_some_and(|c| id.starts_with(c)) {
            return Err(Error(ErrorCode::Unavailable));
        }
        Ok(())
    }
    fn now(&self) -> Vec<ContainerSnapshot> {
        self.members.lock().unwrap().clone()
    }
}
impl Engine for StackEngine {
    async fn inspect(&self, id: &str) -> Result<ContainerSnapshot> {
        self.members
            .lock()
            .unwrap()
            .iter()
            .find(|m| m.resource == format!("container:{id}"))
            .cloned()
            .ok_or(Error(ErrorCode::NotFound))
    }
    async fn restart(&self, id: &str) -> Result<()> {
        self.effect("restart", id)
    }
    async fn start(&self, id: &str) -> Result<()> {
        self.effect("start", id)
    }
    async fn stop(&self, id: &str) -> Result<()> {
        self.effect("stop", id)
    }
    async fn project_members(&self, _: &str) -> Result<Vec<ContainerSnapshot>> {
        Ok(self.now())
    }
}
fn rows(store: &ReceiptStore, table: &str) -> u32 {
    store
        .conn
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}
fn states(receipt: &StackReceipt) -> Vec<(char, ExecutionState)> {
    receipt
        .members
        .iter()
        .map(|m| (m.resource.chars().nth(10).unwrap(), m.state))
        .collect()
}

#[tokio::test]
async fn restart_writes_prepared_rows_for_exactly_the_targets_then_effects_in_id_order() {
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let engine = StackEngine::new();
    let request = request(ContainerAction::Restart);
    let receipt = store
        .execute_stack(&engine, &ceiling(), 1001, &request, || 201)
        .await
        .unwrap();
    assert_eq!(engine.calls(), ["restart a", "restart b"]);
    assert_eq!(
        states(&receipt),
        [
            ('a', ExecutionState::EffectAccepted),
            ('b', ExecutionState::EffectAccepted)
        ]
    );
    assert_eq!(receipt.state, ExecutionState::EffectAccepted);
    // The stopped member was neither sent nor locked.
    assert_eq!(
        (rows(&store, "stack_actions"), rows(&store, "actions")),
        (1, 2)
    );
    assert_eq!(
        store
            .stack_receipt(&request.action, &request.plan_digest)
            .unwrap()
            .unwrap(),
        receipt
    );
    // The member rows are ordinary protected container receipts.
    let a = store
        .receipt(
            &member_action(&request.action, &snap('a', true).resource),
            &request.plan_digest,
        )
        .unwrap()
        .unwrap();
    assert_eq!(a.state, ExecutionState::EffectAccepted);
    assert_eq!(a.before, snap('a', true));
}

#[tokio::test]
async fn start_and_stop_select_targets_by_state_and_use_their_own_engine_call() {
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let engine = StackEngine::new();
    let mut start = request(ContainerAction::Start);
    start.action = "e".repeat(64);
    start.plan_digest = limeos_identity::digest(&encode(&start.plan).unwrap());
    store
        .execute_stack(&engine, &ceiling(), 1001, &start, || 201)
        .await
        .unwrap();
    assert_eq!(engine.calls(), ["start c"]);
    // The start action is unresolved and holds the stack; use a fresh store.
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let engine = StackEngine::new();
    let stop = request(ContainerAction::Stop);
    store
        .execute_stack(&engine, &ceiling(), 1001, &stop, || 201)
        .await
        .unwrap();
    assert_eq!(engine.calls(), ["stop a", "stop b"]);
}

#[tokio::test]
async fn targets_are_verified_independently_and_release_only_their_own_locks() {
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let engine = StackEngine::new();
    let request = request(ContainerAction::Restart);
    store
        .execute_stack(&engine, &ceiling(), 1001, &request, || 201)
        .await
        .unwrap();
    let after = engine.now();
    // Only one member's proof is supplied: that member alone is released.
    let partial = store
        .verify_stack(
            &engine,
            &ceiling(),
            1001,
            &request.action,
            &request.plan_digest,
            &after[..1],
        )
        .await
        .unwrap();
    assert_eq!(
        states(&partial),
        [
            ('a', ExecutionState::Verified),
            ('b', ExecutionState::EffectAccepted)
        ]
    );
    assert_eq!(partial.state, ExecutionState::EffectAccepted);
    // A forged post-state never verifies, and the Engine is asked again.
    let mut forged = after[1].clone();
    forged.started_at = "2026-10-05T07:59:00Z".into();
    let unchanged = store
        .verify_stack(
            &engine,
            &ceiling(),
            1001,
            &request.action,
            &request.plan_digest,
            &[forged],
        )
        .await
        .unwrap();
    assert_eq!(unchanged.state, ExecutionState::EffectAccepted);
    let done = store
        .verify_stack(
            &engine,
            &ceiling(),
            1001,
            &request.action,
            &request.plan_digest,
            &after,
        )
        .await
        .unwrap();
    assert_eq!(done.state, ExecutionState::Verified);
    // The completed stack and its member locks are gone: single actions resume.
    let single = single_restart(&after[0]);
    store
        .execute(&engine, &ceiling(), 1001, &single, || 201)
        .await
        .unwrap();
}

#[tokio::test]
async fn single_container_actions_and_other_stack_actions_cannot_share_a_member() {
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let engine = StackEngine::new();
    let first = request(ContainerAction::Restart);
    store
        .execute_stack(&engine, &ceiling(), 1001, &first, || 201)
        .await
        .unwrap();
    let before = rows(&store, "actions");
    let effects = engine.calls().len();
    // A single-container job on a locked target is refused before its effect.
    let single = single_restart(&engine.now()[0]);
    assert_eq!(
        store
            .execute(&engine, &ceiling(), 1001, &single, || 201)
            .await
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    // A second stack action on the same project and one on an overlapping
    // member are both refused while the first is unresolved.
    let mut second = request_for(ContainerAction::Stop, engine.now());
    second.action = "8".repeat(64);
    assert_eq!(
        store
            .execute_stack(&engine, &ceiling(), 1001, &second, || 201)
            .await
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    assert_eq!(engine.calls().len(), effects);
    assert_eq!(rows(&store, "actions"), before);
    assert_eq!(rows(&store, "stack_actions"), 1);
}

#[tokio::test]
async fn partial_failure_is_fail_stop_records_each_member_and_tolerates_partial_verification() {
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let all_running = vec![snap('a', true), snap('b', true), snap('c', true)];
    let engine = StackEngine::with(all_running.clone(), Some('b'));
    let request = request_for(ContainerAction::Restart, all_running);
    let receipt = store
        .execute_stack(&engine, &ceiling(), 1001, &request, || 201)
        .await
        .unwrap();
    assert_eq!(engine.calls(), ["restart a", "restart b"]);
    assert_eq!(
        states(&receipt),
        [
            ('a', ExecutionState::EffectAccepted),
            ('b', ExecutionState::OutcomeUnknown),
            ('c', ExecutionState::PreconditionChanged)
        ]
    );
    assert_eq!(receipt.members[1].error, Some(ErrorCode::Unavailable));
    assert_eq!(receipt.state, ExecutionState::OutcomeUnknown);
    // Persisted exactly as returned, including the per-member evidence.
    assert_eq!(
        store
            .stack_receipt(&request.action, &request.plan_digest)
            .unwrap()
            .unwrap(),
        receipt
    );
    let after = engine.now();
    // a verifies and is released; b is proven by fresh inspection even though
    // its response was lost; c was never sent and stays so.
    // First only a is proven: b is still unknown, so the stack stays locked.
    let first = store
        .verify_stack(
            &engine,
            &ceiling(),
            1001,
            &request.action,
            &request.plan_digest,
            &after[..1],
        )
        .await
        .unwrap();
    assert_eq!(first.state, ExecutionState::OutcomeUnknown);
    let mut single = single_restart(&after[0]);
    single.action = "9".repeat(64);
    single.plan_digest = limeos_identity::digest(&encode(&single.plan).unwrap());
    let mut next = request_for(ContainerAction::Restart, engine.now());
    next.action = "5".repeat(64);
    assert_eq!(
        store
            .execute_stack(&engine, &ceiling(), 1001, &next, || 201)
            .await
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    let verified = store
        .verify_stack(
            &engine,
            &ceiling(),
            1001,
            &request.action,
            &request.plan_digest,
            &after,
        )
        .await
        .unwrap();
    assert_eq!(
        states(&verified),
        [
            ('a', ExecutionState::Verified),
            ('b', ExecutionState::Verified),
            ('c', ExecutionState::PreconditionChanged)
        ]
    );
    // Every sent member is proven and the rest were never sent: terminal, so
    // the locks are released, but it is not a success.
    assert_eq!(verified.state, ExecutionState::PartlyDone);
    assert_eq!(engine.calls().len(), 2);
    let persisted = store
        .stack_receipt(&request.action, &request.plan_digest)
        .unwrap()
        .unwrap();
    assert_eq!(persisted, verified);
    next.plan = {
        let mut plan = next.plan.clone();
        plan.expected = StackSnapshot::new("media", engine.now()).unwrap();
        plan
    };
    next.plan_digest = limeos_identity::digest(&encode(&next.plan).unwrap());
    let released = store
        .execute_stack(&engine, &ceiling(), 1001, &next, || 201)
        .await
        .unwrap();
    assert_eq!(released.members[0].state, ExecutionState::EffectAccepted);
}

/// Changes one member just before its own Engine call, like a concurrent
/// operator action during a long stack run.
struct Racing {
    inner: StackEngine,
    change_after: usize,
    target: String,
}
impl Engine for Racing {
    async fn inspect(&self, id: &str) -> Result<ContainerSnapshot> {
        let mut value = self.inner.inspect(id).await?;
        if self.inner.calls().len() >= self.change_after && id.starts_with(&self.target) {
            value.started_at = "2026-10-05T08:30:00Z".into();
        }
        Ok(value)
    }
    async fn restart(&self, id: &str) -> Result<()> {
        self.inner.restart(id).await
    }
    async fn project_members(&self, p: &str) -> Result<Vec<ContainerSnapshot>> {
        self.inner.project_members(p).await
    }
}

#[tokio::test]
async fn a_target_that_changed_during_the_run_is_not_sent_and_ends_the_run() {
    for (change_after, sent) in [(0, 0), (1, 1)] {
        let dir = private_dir();
        let mut store = ReceiptStore::open(dir.path()).unwrap();
        let engine = Racing {
            inner: StackEngine::new(),
            change_after,
            target: if change_after == 0 { "a" } else { "b" }.into(),
        };
        let request = request(ContainerAction::Restart);
        let receipt = store
            .execute_stack(&engine, &ceiling(), 1001, &request, || 201)
            .await
            .unwrap();
        assert_eq!(engine.inner.calls().len(), sent);
        assert_eq!(receipt.error, Some(ErrorCode::Conflict));
        let moved = &receipt.members[sent];
        assert_eq!(moved.state, ExecutionState::PreconditionChanged);
        if sent == 0 {
            // Nothing sent at all: terminal, no lock.
            assert_eq!(receipt.state, ExecutionState::PreconditionChanged);
        } else {
            assert_eq!(receipt.members[0].state, ExecutionState::EffectAccepted);
            assert_eq!(receipt.state, ExecutionState::OutcomeUnknown);
        }
        assert_eq!(
            store
                .stack_receipt(&request.action, &request.plan_digest)
                .unwrap()
                .unwrap(),
            receipt
        );
    }
}

#[tokio::test]
async fn member_drift_is_refused_whole_without_effects_locks_or_a_replay_hole() {
    for drift in 0..4 {
        let dir = private_dir();
        let mut store = ReceiptStore::open(dir.path()).unwrap();
        let mut actual = members();
        match drift {
            0 => actual.push(snap('d', true)),
            1 => {
                actual.pop();
            }
            2 => actual[0].started_at = "2026-10-05T07:09:00Z".into(),
            _ => actual[2].running = true,
        }
        let engine = StackEngine::with(actual, None);
        let request = request(ContainerAction::Restart);
        let receipt = store
            .execute_stack(&engine, &ceiling(), 1001, &request, || 201)
            .await
            .unwrap();
        assert!(engine.calls().is_empty(), "drift {drift}");
        assert_eq!(receipt.state, ExecutionState::PreconditionChanged);
        assert_eq!(receipt.error, Some(ErrorCode::Conflict));
        // No lock is held, and the same action never becomes effective later.
        let healed = StackEngine::new();
        let again = store
            .execute_stack(&healed, &ceiling(), 1001, &request, || 202)
            .await
            .unwrap();
        assert_eq!(again, receipt);
        assert!(healed.calls().is_empty());
        let mut next = request.clone();
        next.action = "7".repeat(64);
        let fresh = store
            .execute_stack(&healed, &ceiling(), 1001, &next, || 203)
            .await
            .unwrap();
        assert_eq!(fresh.state, ExecutionState::EffectAccepted);
    }
}

#[tokio::test]
async fn expired_plans_and_bad_digests_are_refused_before_any_engine_contact() {
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let engine = StackEngine::new();
    let request = request(ContainerAction::Restart);
    assert_eq!(
        store
            .execute_stack(&engine, &ceiling(), 1001, &request, || 500)
            .await
            .unwrap_err()
            .0,
        ErrorCode::Expired
    );
    let mut bad = request.clone();
    bad.plan_digest = "0".repeat(64);
    assert_eq!(
        store
            .execute_stack(&engine, &ceiling(), 1001, &bad, || 201)
            .await
            .unwrap_err()
            .0,
        ErrorCode::InvalidInput
    );
    assert!(engine.calls().is_empty());
    assert_eq!(rows(&store, "stack_actions"), 0);
}

/// Every execute path checks the plan's own `expires_at` twice: before it
/// prepares, and after the durable Prepared row, immediately before the
/// Engine call. Core no longer relies on this to release a job without a
/// receipt (a closure does that, whatever the clock says; see `closure`), but
/// it still keeps an expired plan from acting.
#[tokio::test]
async fn every_execute_path_refuses_its_plan_from_expiry_before_and_after_preparing() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let stack = request(ContainerAction::Restart);
    let single = single_restart(&snap('a', true));
    assert_eq!(stack.plan.expires_at, single.plan.expires_at);
    let expires_at = stack.plan.expires_at;
    let settle = expires_at + 5;
    // A request that reaches the store at or after expiry: refused before
    // any Engine contact, and no row is written.
    for at in [expires_at, settle, expires_at + 3600] {
        let dir = private_dir();
        let mut store = ReceiptStore::open(dir.path()).unwrap();
        let engine = StackEngine::new();
        let refused = store
            .execute_stack(&engine, &ceiling(), 1001, &stack, || at)
            .await
            .unwrap_err();
        assert_eq!(refused.0, ErrorCode::Expired);
        let refused = store
            .execute(&engine, &ceiling(), 1001, &single, || at)
            .await
            .unwrap_err();
        assert_eq!(refused.0, ErrorCode::Expired);
        assert!(engine.calls().is_empty());
        assert_eq!(
            (rows(&store, "stack_actions"), rows(&store, "actions")),
            (0, 0)
        );
    }
    // A plan that expires while its Prepared row is synced: the second check
    // records the refusal and nothing reaches the Engine. Every clock read up
    // to and including the insert is valid; the next one is expired.
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let engine = StackEngine::new();
    let reads = AtomicUsize::new(0);
    let clock = || {
        if reads.fetch_add(1, Ordering::SeqCst) < 3 {
            expires_at - 1
        } else {
            expires_at
        }
    };
    let receipt = store
        .execute_stack(&engine, &ceiling(), 1001, &stack, clock)
        .await
        .unwrap();
    assert_eq!(reads.load(Ordering::SeqCst), 4);
    assert_eq!(receipt.state, ExecutionState::PreconditionChanged);
    assert_eq!(receipt.error, Some(ErrorCode::Expired));
    assert!(
        receipt
            .members
            .iter()
            .all(|m| m.state == ExecutionState::PreconditionChanged)
    );
    assert!(engine.calls().is_empty());
    let reads = AtomicUsize::new(0);
    let clock = || {
        if reads.fetch_add(1, Ordering::SeqCst) < 3 {
            expires_at - 1
        } else {
            expires_at
        }
    };
    let receipt = store
        .execute(&engine, &ceiling(), 1001, &single, clock)
        .await
        .unwrap();
    assert_eq!(reads.load(Ordering::SeqCst), 4);
    assert_eq!(receipt.state, ExecutionState::PreconditionChanged);
    assert_eq!(receipt.error, Some(ErrorCode::Expired));
    assert!(engine.calls().is_empty());
}

#[tokio::test]
async fn authorization_failures_write_nothing_and_never_reach_the_engine() {
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let engine = StackEngine::new();
    let request = request(ContainerAction::Restart);
    let change = |f: &dyn Fn(&mut ContainerCeiling)| {
        let mut c = ceiling();
        f(&mut c);
        c
    };
    for (peer, policy) in [
        (1002, ceiling()),
        (1001, change(&|c| c.allow_stack = false)),
        (1001, change(&|c| c.allow_restart = false)),
        (1001, change(&|c| c.managed_stacks = vec!["other".into()])),
        (1001, change(&|c| c.managed_stacks.clear())),
        (1001, change(&|c| c.managed_containers.truncate(1))),
        (
            1001,
            change(&|c| c.managed_stacks = vec!["../media".into()]),
        ),
    ] {
        assert_eq!(
            store
                .execute_stack(&engine, &policy, peer, &request, || 201)
                .await
                .unwrap_err()
                .0,
            ErrorCode::Forbidden
        );
    }
    assert!(engine.calls().is_empty());
    assert_eq!(
        (rows(&store, "stack_actions"), rows(&store, "actions")),
        (0, 0)
    );
    assert_eq!(
        store
            .verify_stack(
                &engine,
                &ceiling(),
                1001,
                &"1".repeat(64),
                &"2".repeat(64),
                &[]
            )
            .await
            .unwrap_err()
            .0,
        ErrorCode::NotFound
    );
}

#[tokio::test]
async fn replay_by_action_returns_the_stored_receipt_without_any_effect() {
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let engine = StackEngine::new();
    let request = request(ContainerAction::Restart);
    let first = store
        .execute_stack(&engine, &ceiling(), 1001, &request, || 201)
        .await
        .unwrap();
    let again = store
        .execute_stack(&engine, &ceiling(), 1001, &request, || 9999)
        .await
        .unwrap();
    assert_eq!(first, again);
    assert_eq!(engine.calls().len(), 2);
    // Same action with a different plan digest is a conflict, not a new effect.
    let mut other = request.clone();
    other.plan.operation = ContainerAction::Stop;
    other.plan_digest = limeos_identity::digest(&encode(&other.plan).unwrap());
    assert_eq!(
        store
            .execute_stack(&engine, &ceiling(), 1001, &other, || 201)
            .await
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    assert_eq!(engine.calls().len(), 2);
}

#[tokio::test]
async fn tampered_member_or_stack_rows_are_reported_as_not_durable() {
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let engine = StackEngine::new();
    let request = request(ContainerAction::Restart);
    store
        .execute_stack(&engine, &ceiling(), 1001, &request, || 201)
        .await
        .unwrap();
    store
        .conn
        .execute(
            "UPDATE actions SET state='verified' WHERE resource=?",
            [snap('a', true).resource],
        )
        .unwrap();
    assert_eq!(
        store
            .stack_receipt(&request.action, &request.plan_digest)
            .unwrap_err()
            .0,
        ErrorCode::StateNotDurable
    );
}

#[test]
fn schema_three_upgrade_adds_the_stack_table_and_keeps_container_receipts() {
    let dir = private_dir();
    let store = ReceiptStore::open(dir.path()).unwrap();
    store
        .conn
        .execute_batch("DROP TABLE retention; DROP TABLE pruned_legacy; DROP TABLE stack_actions; PRAGMA user_version=3;")
        .unwrap();
    drop(store);
    let store = ReceiptStore::open(dir.path()).unwrap();
    let version: u32 = store
        .conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, crate::RECEIPT_SCHEMA_VERSION);
    assert_eq!(rows(&store, "stack_actions"), 0);
}

/// Exits the process at the chosen Engine call, after making the host effect
/// durable in a side file, like a power loss between two physical effects.
struct CrashEngine {
    directory: std::path::PathBuf,
    crash_at: usize,
}
impl CrashEngine {
    fn effect(&self, id: &str) -> Result<()> {
        use std::io::Write;
        let path = self.directory.join("effects");
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(file, "{}", &id[..1]).unwrap();
        file.sync_all().unwrap();
        let count = std::fs::read_to_string(&path).unwrap().lines().count();
        if count == self.crash_at {
            std::process::exit(86);
        }
        Ok(())
    }
}
impl Engine for CrashEngine {
    async fn inspect(&self, id: &str) -> Result<ContainerSnapshot> {
        members()
            .into_iter()
            .find(|m| m.resource == format!("container:{id}"))
            .ok_or(Error(ErrorCode::NotFound))
    }
    async fn restart(&self, id: &str) -> Result<()> {
        self.effect(id)
    }
    async fn project_members(&self, _: &str) -> Result<Vec<ContainerSnapshot>> {
        Ok(members())
    }
}
#[test]
fn stack_crash_helper() {
    let Some(directory) = std::env::var_os("LIMEOS_STACK_CRASH_FIXTURE") else {
        return;
    };
    let crash_at = std::env::var("LIMEOS_STACK_CRASH_AT")
        .unwrap()
        .parse()
        .unwrap();
    let directory = std::path::PathBuf::from(directory);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut store = ReceiptStore::open(&directory).unwrap();
        let _ = store
            .execute_stack(
                &CrashEngine {
                    directory,
                    crash_at,
                },
                &ceiling(),
                1001,
                &request(ContainerAction::Restart),
                || 201,
            )
            .await;
    });
    panic!("crash fixture must exit during an effect");
}
fn crash_after(effects: usize) -> tempfile::TempDir {
    let dir = private_dir();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "stack::tests::stack_crash_helper", "--nocapture"])
        .env("LIMEOS_STACK_CRASH_FIXTURE", dir.path())
        .env("LIMEOS_STACK_CRASH_AT", effects.to_string())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(86));
    dir
}

#[tokio::test]
async fn crash_before_the_first_effect_result_never_replays_and_stays_uncertain() {
    // The first Engine call made its host effect, then the process died before
    // any member result was persisted: every member is still Prepared.
    let dir = crash_after(1);
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let request = request(ContainerAction::Restart);
    let engine = StackEngine::new();
    // Re-delivery of the same action returns the stored Prepared receipt.
    let again = store
        .execute_stack(&engine, &ceiling(), 1001, &request, || 202)
        .await
        .unwrap();
    assert_eq!(again.state, ExecutionState::Prepared);
    assert!(engine.calls().is_empty());
    // Recovery lookup converts unresolved Prepared members to uncertain.
    let recovered = store
        .reconcile_stack(&request.action, &request.plan_digest)
        .unwrap()
        .unwrap();
    assert_eq!(recovered.state, ExecutionState::OutcomeUnknown);
    assert!(
        recovered
            .members
            .iter()
            .all(|m| m.state == ExecutionState::OutcomeUnknown
                && m.error == Some(ErrorCode::StateNotDurable))
    );
    // Locks stay held: neither a single action nor a new stack action runs.
    let single = single_restart(&snap('a', true));
    assert!(
        store
            .execute(&engine, &ceiling(), 1001, &single, || 203)
            .await
            .is_err()
    );
    let mut second = request.clone();
    second.action = "8".repeat(64);
    assert!(
        store
            .execute_stack(&engine, &ceiling(), 1001, &second, || 203)
            .await
            .is_err()
    );
    assert!(engine.calls().is_empty());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("effects")).unwrap(),
        "a\n"
    );
    // A fresh inspection of the already-restarted first target may prove it;
    // the second was never sent and stays uncertain.
    let mut after = members();
    after[0].started_at = "2026-10-05T07:05:00Z".into();
    let engine = StackEngine::with(after.clone(), None);
    let verified = store
        .verify_stack(
            &engine,
            &ceiling(),
            1001,
            &request.action,
            &request.plan_digest,
            &after[..2],
        )
        .await
        .unwrap();
    assert_eq!(verified.members[0].state, ExecutionState::Verified);
    assert_eq!(verified.members[1].state, ExecutionState::OutcomeUnknown);
    assert!(engine.calls().is_empty());
}

#[tokio::test]
async fn crash_between_two_effects_keeps_the_first_result_and_never_sends_the_second() {
    let dir = crash_after(2);
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let request = request(ContainerAction::Restart);
    let engine = StackEngine::new();
    let found = store
        .stack_receipt(&request.action, &request.plan_digest)
        .unwrap()
        .unwrap();
    // Member a's result was durable; b is Prepared (sent or not is unknown).
    assert_eq!(
        states(&found),
        [
            ('a', ExecutionState::EffectAccepted),
            ('b', ExecutionState::Prepared)
        ]
    );
    assert_eq!(found.state, ExecutionState::OutcomeUnknown);
    let recovered = store
        .reconcile_stack(&request.action, &request.plan_digest)
        .unwrap()
        .unwrap();
    assert_eq!(
        states(&recovered),
        [
            ('a', ExecutionState::EffectAccepted),
            ('b', ExecutionState::OutcomeUnknown)
        ]
    );
    assert!(
        store
            .execute_stack(&engine, &ceiling(), 1001, &request, || 210)
            .await
            .unwrap()
            == recovered
    );
    assert!(engine.calls().is_empty());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("effects")).unwrap(),
        "a\nb\n"
    );
    // Reopening the store (a second restart) keeps the same evidence.
    drop(store);
    let store = ReceiptStore::open(dir.path()).unwrap();
    assert_eq!(
        store
            .stack_receipt(&request.action, &request.plan_digest)
            .unwrap()
            .unwrap(),
        recovered
    );
}
#[test]
fn a_stack_is_authorized_only_for_core_and_never_for_a_root_core() {
    let expected = request(ContainerAction::Restart).plan.expected;
    assert!(
        ceiling()
            .authorize_stack(1001, &expected, ContainerAction::Restart)
            .is_ok()
    );
    assert_eq!(
        ceiling().authorize_stack(1002, &expected, ContainerAction::Restart),
        Err(Error(ErrorCode::Forbidden))
    );
    let root = ContainerCeiling {
        core_uid: 0,
        ..ceiling()
    };
    assert_eq!(
        root.authorize_stack(0, &expected, ContainerAction::Restart),
        Err(Error(ErrorCode::Forbidden))
    );
}
