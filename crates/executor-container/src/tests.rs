use super::*;
use limeos_domain::RestartPlan;

#[tokio::test]
async fn accepted_result_retains_lock_until_fresh_independent_verification() {
    let directory = private_dir();
    let mut store = ReceiptStore::open(directory.path()).unwrap();
    let mut engine = FakeEngine::new(Mode::Accept);
    let request = request();
    store
        .execute(&engine, &ceiling(), 1001, &request, || 201)
        .await
        .unwrap();
    let mut next = request.clone();
    next.action = "e".repeat(64);
    assert!(
        store
            .execute(&engine, &ceiling(), 1001, &next, || 201)
            .await
            .is_err()
    );
    assert!(
        store
            .verify(
                &engine,
                &ceiling(),
                1001,
                &request.action,
                &request.plan_digest,
                &request.plan.expected
            )
            .await
            .is_err()
    );
    engine.snapshot.started_at = "2026-10-05T07:01:00Z".into();
    assert_eq!(
        store
            .verify(
                &engine,
                &ceiling(),
                1001,
                &request.action,
                &request.plan_digest,
                &engine.snapshot
            )
            .await
            .unwrap()
            .state,
        ExecutionState::Verified
    );
    next.plan.expected = engine.snapshot.clone();
    next.plan_digest = limeos_identity::digest(&encode(&next.plan).unwrap());
    assert!(
        store
            .execute(&engine, &ceiling(), 1001, &next, || 201)
            .await
            .is_ok()
    );
}
use std::sync::atomic::{AtomicUsize, Ordering};

fn private_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}

fn expected() -> ContainerSnapshot {
    ContainerSnapshot {
        resource: format!("container:{}", "a".repeat(64)),
        image: format!("sha256:{}", "b".repeat(64)),
        started_at: "2026-10-05T07:00:00.000000000Z".into(),
        running: true,
    }
}
fn request() -> RestartRequest {
    let plan = RestartPlan {
        operation: limeos_domain::ContainerAction::Restart,
        id: "c".repeat(64),
        version: 1,
        principal: "alice".into(),
        grant_revision: 1,
        expected: expected(),
        created_at: 200,
        expires_at: 500,
    };
    RestartRequest {
        action: "d".repeat(64),
        plan_digest: limeos_identity::digest(&encode(&plan).unwrap()),
        plan,
    }
}
fn ceiling() -> RestartCeiling {
    RestartCeiling {
        version: 1,
        core_uid: 1001,
        allow_restart: true,
        allow_start: false,
        allow_stop: false,
        allow_stack: false,
        managed_containers: vec!["a".repeat(64)],
        managed_stacks: Vec::new(),
    }
}
#[derive(Clone, Copy)]
enum Mode {
    Accept,
    Fail,
    Hang,
}
struct FakeEngine {
    effects: AtomicUsize,
    snapshot: ContainerSnapshot,
    mode: Mode,
}
impl FakeEngine {
    fn new(mode: Mode) -> Self {
        Self {
            effects: AtomicUsize::new(0),
            snapshot: expected(),
            mode,
        }
    }
}
impl Engine for FakeEngine {
    async fn inspect(&self, _: &str) -> Result<ContainerSnapshot> {
        Ok(self.snapshot.clone())
    }
    async fn restart(&self, id: &str) -> Result<()> {
        assert_eq!(id, "a".repeat(64));
        self.effects.fetch_add(1, Ordering::SeqCst);
        match self.mode {
            Mode::Accept => Ok(()),
            Mode::Fail => Err(Error(ErrorCode::Unavailable)),
            Mode::Hang => std::future::pending().await,
        }
    }
    async fn start(&self, id: &str) -> Result<()> {
        self.restart(id).await
    }
    async fn stop(&self, id: &str) -> Result<()> {
        self.restart(id).await
    }
}
#[tokio::test]
async fn each_lifecycle_action_requires_its_own_ceiling_and_verified_result() {
    for operation in [ContainerAction::Start, ContainerAction::Stop] {
        let directory = private_dir();
        let mut store = ReceiptStore::open(directory.path()).unwrap();
        let mut engine = FakeEngine::new(Mode::Accept);
        engine.snapshot.running = operation != ContainerAction::Start;
        let mut req = request();
        req.plan.operation = operation;
        req.plan.expected = engine.snapshot.clone();
        req.plan_digest = limeos_identity::digest(&encode(&req.plan).unwrap());
        assert_eq!(
            store
                .execute(&engine, &ceiling(), 1001, &req, || 201)
                .await
                .unwrap_err()
                .0,
            ErrorCode::Forbidden
        );
        assert_eq!(engine.effects.load(Ordering::SeqCst), 0);
        let enabled = ContainerCeiling {
            allow_start: operation == ContainerAction::Start,
            allow_stop: operation == ContainerAction::Stop,
            ..ceiling()
        };
        let receipt = store
            .execute(&engine, &enabled, 1001, &req, || 201)
            .await
            .unwrap();
        assert_eq!(receipt.operation, operation);
        assert!(
            store
                .verify(
                    &engine,
                    &enabled,
                    1001,
                    &req.action,
                    &req.plan_digest,
                    &engine.snapshot
                )
                .await
                .is_err()
        );
        if operation == ContainerAction::Start {
            engine.snapshot.running = true;
            engine.snapshot.started_at = "2026-10-05T07:01:00Z".into();
        } else {
            engine.snapshot.running = false;
        }
        assert_eq!(
            store
                .verify(
                    &engine,
                    &enabled,
                    1001,
                    &req.action,
                    &req.plan_digest,
                    &engine.snapshot
                )
                .await
                .unwrap()
                .state,
            ExecutionState::Verified
        );
        store
            .execute(&engine, &enabled, 1001, &req, || 999)
            .await
            .unwrap();
        assert_eq!(engine.effects.load(Ordering::SeqCst), 1);
    }
}
#[tokio::test]
async fn v2_receipt_migration_retains_the_prepared_barrier_without_replay() {
    let directory = private_dir();
    let mut store = ReceiptStore::open(directory.path()).unwrap();
    let req = request();
    store
        .insert_action(
            &ContainerReceipt {
                action: req.action.clone(),
                plan_digest: req.plan_digest.clone(),
                operation: ContainerAction::Restart,
                before: req.plan.expected.clone(),
                state: ExecutionState::Prepared,
                error: None,
            },
            req.plan.expires_at,
            201,
            false,
        )
        .unwrap();
    store
        .conn
        .execute_batch("DROP TABLE effect_log; DROP TABLE effect_days; DROP TABLE retention; DROP TABLE pruned_legacy; DROP INDEX actions_digest; ALTER TABLE actions DROP COLUMN expires_at; DROP TABLE stack_actions; ALTER TABLE actions DROP COLUMN operation; PRAGMA user_version=2;")
        .unwrap();
    drop(store);
    let mut store = ReceiptStore::open(directory.path()).unwrap();
    let engine = FakeEngine::new(Mode::Accept);
    assert_eq!(
        store
            .execute(&engine, &ceiling(), 1001, &req, || 201)
            .await
            .unwrap()
            .state,
        ExecutionState::Prepared
    );
    assert_eq!(engine.effects.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn peer_ceiling_plan_and_fresh_precondition_reject_before_any_effect() {
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let engine = FakeEngine::new(Mode::Accept);
    let req = request();
    assert_eq!(
        store
            .execute(&engine, &ceiling(), 1002, &req, || 201)
            .await
            .unwrap_err()
            .0,
        ErrorCode::Forbidden
    );
    for policy in [
        RestartCeiling {
            allow_restart: false,
            ..ceiling()
        },
        RestartCeiling {
            managed_containers: Vec::new(),
            ..ceiling()
        },
        RestartCeiling {
            managed_containers: vec!["../docker.sock".into()],
            ..ceiling()
        },
    ] {
        assert_eq!(
            store
                .execute(&engine, &policy, 1001, &req, || 201)
                .await
                .unwrap_err()
                .0,
            ErrorCode::Forbidden
        );
    }
    let mut altered = request();
    altered.plan.expected.image = format!("sha256:{}", "e".repeat(64));
    assert_eq!(
        store
            .execute(&engine, &ceiling(), 1001, &altered, || 201)
            .await
            .unwrap_err()
            .0,
        ErrorCode::InvalidInput
    );
    assert_eq!(
        store
            .execute(&engine, &ceiling(), 1001, &req, || 500)
            .await
            .unwrap_err()
            .0,
        ErrorCode::Expired
    );
    let changed = FakeEngine {
        snapshot: ContainerSnapshot {
            running: false,
            ..expected()
        },
        ..FakeEngine::new(Mode::Accept)
    };
    assert_eq!(
        store
            .execute(&changed, &ceiling(), 1001, &req, || 201)
            .await
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
    assert_eq!(engine.effects.load(Ordering::SeqCst), 0);
    assert!(
        store
            .receipt(&req.action, &req.plan_digest)
            .unwrap()
            .is_none()
    );
    let mut value = serde_json::to_value(req).unwrap();
    for key in ["shell", "command", "path", "risk", "principal"] {
        value[key] = serde_json::json!("injected");
        assert!(serde_json::from_value::<RestartRequest>(value.clone()).is_err());
        value.as_object_mut().unwrap().remove(key);
    }
}

#[tokio::test]
async fn accepted_effect_is_not_claimed_verified_and_repeat_action_returns_original_receipt() {
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let engine = FakeEngine::new(Mode::Accept);
    let req = request();
    let receipt = store
        .execute(&engine, &ceiling(), 1001, &req, || 201)
        .await
        .unwrap();
    assert_eq!(receipt.state, ExecutionState::EffectAccepted);
    assert_eq!(
        store
            .execute(&engine, &ceiling(), 1001, &req, || 600)
            .await
            .unwrap(),
        receipt
    );
    assert_eq!(engine.effects.load(Ordering::SeqCst), 1);
    let mut substituted = request();
    substituted.plan.id = "f".repeat(64);
    substituted.plan_digest = limeos_identity::digest(&encode(&substituted.plan).unwrap());
    assert_eq!(
        store
            .execute(&engine, &ceiling(), 1001, &substituted, || 202)
            .await
            .unwrap_err()
            .0,
        ErrorCode::Conflict
    );
}

#[tokio::test(start_paused = true)]
async fn uncertain_or_timed_out_effect_keeps_a_resource_barrier_after_executor_restart() {
    for mode in [Mode::Fail, Mode::Hang] {
        let dir = private_dir();
        let mut store = ReceiptStore::open(dir.path()).unwrap();
        let engine = FakeEngine::new(mode);
        let req = request();
        assert_eq!(
            store
                .execute(&engine, &ceiling(), 1001, &req, || 201)
                .await
                .unwrap()
                .state,
            ExecutionState::OutcomeUnknown
        );
        drop(store);
        let mut recovered = ReceiptStore::open(dir.path()).unwrap();
        assert_eq!(
            recovered
                .execute(&engine, &ceiling(), 1001, &req, || 202)
                .await
                .unwrap()
                .state,
            ExecutionState::OutcomeUnknown
        );
        let next = RestartRequest {
            action: "e".repeat(64),
            ..request()
        };
        assert!(
            recovered
                .execute(&engine, &ceiling(), 1001, &next, || 203)
                .await
                .is_err()
        );
        assert_eq!(engine.effects.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn storage_failure_before_prepare_has_no_effect_and_failure_after_effect_stays_unknown() {
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let engine = FakeEngine::new(Mode::Accept);
    let req = request();
    store.conn.execute_batch("CREATE TRIGGER prevent_prepare BEFORE INSERT ON actions BEGIN SELECT RAISE(FAIL,'fixture disk full'); END;").unwrap();
    assert_eq!(
        store
            .execute(&engine, &ceiling(), 1001, &req, || 201)
            .await
            .unwrap_err()
            .0,
        ErrorCode::StateNotDurable
    );
    assert_eq!(engine.effects.load(Ordering::SeqCst), 0);
    store.conn.execute_batch("DROP TRIGGER prevent_prepare; CREATE TRIGGER prevent_finish BEFORE UPDATE ON actions BEGIN SELECT RAISE(FAIL,'fixture disk full'); END;").unwrap();
    let receipt = store
        .execute(&engine, &ceiling(), 1001, &req, || 202)
        .await
        .unwrap();
    assert_eq!(receipt.state, ExecutionState::OutcomeUnknown);
    assert_eq!(receipt.error, Some(ErrorCode::StateNotDurable));
    assert_eq!(
        store
            .receipt(&req.action, &req.plan_digest)
            .unwrap()
            .unwrap()
            .state,
        ExecutionState::Prepared
    );
    assert_eq!(
        store
            .execute(&engine, &ceiling(), 1001, &req, || 203)
            .await
            .unwrap()
            .state,
        ExecutionState::Prepared
    );
    assert_eq!(engine.effects.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn expiry_during_receipt_sync_prevents_dispatch() {
    let dir = private_dir();
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let engine = FakeEngine::new(Mode::Accept);
    let reads = AtomicUsize::new(0);
    let clock = || {
        if reads.fetch_add(1, Ordering::SeqCst) < 2 {
            201
        } else {
            500
        }
    };
    let receipt = store
        .execute(&engine, &ceiling(), 1001, &request(), clock)
        .await
        .unwrap();
    assert_eq!(receipt.state, ExecutionState::PreconditionChanged);
    assert_eq!(receipt.error, Some(ErrorCode::Expired));
    assert_eq!(engine.effects.load(Ordering::SeqCst), 0);
}

#[test]
fn receipt_paths_are_private_single_owner_and_fail_closed_on_links_or_future_schema() {
    let dir = private_dir();
    let store = ReceiptStore::open(dir.path()).unwrap();
    assert!(ReceiptStore::open(dir.path()).is_err());
    assert_eq!(
        std::fs::metadata(dir.path().join("receipts.sqlite"))
            .unwrap()
            .mode()
            & 0o777,
        0o600
    );
    let version: u32 = store
        .conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, RECEIPT_SCHEMA_VERSION);
    store
        .conn
        .execute_batch(&format!(
            "PRAGMA user_version={};",
            RECEIPT_SCHEMA_VERSION + 1
        ))
        .unwrap();
    drop(store);
    assert!(ReceiptStore::open(dir.path()).is_err());
    let unsafe_dir = private_dir();
    std::fs::set_permissions(unsafe_dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(ReceiptStore::open(unsafe_dir.path()).is_err());
    let linked = private_dir();
    std::os::unix::fs::symlink(
        dir.path().join("receipts.sqlite"),
        linked.path().join("receipts.sqlite"),
    )
    .unwrap();
    assert!(ReceiptStore::open(linked.path()).is_err());
}

struct CrashEngine {
    directory: std::path::PathBuf,
}
impl Engine for CrashEngine {
    async fn inspect(&self, _: &str) -> Result<ContainerSnapshot> {
        Ok(expected())
    }
    async fn restart(&self, _: &str) -> Result<()> {
        use std::io::Write;
        // This file stands for a separately durable host effect in the fixture.
        let mut file = File::create(self.directory.join("effect-count")).unwrap();
        file.write_all(b"1").unwrap();
        file.sync_all().unwrap();
        File::open(&self.directory).unwrap().sync_all().unwrap();
        std::process::exit(86);
    }
}
#[test]
fn crash_helper() {
    let Some(directory) = std::env::var_os("LIMEOS_RECEIPT_CRASH_FIXTURE") else {
        return;
    };
    let directory = std::path::PathBuf::from(directory);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut store = ReceiptStore::open(&directory).unwrap();
        let _ = store
            .execute(
                &CrashEngine { directory },
                &ceiling(),
                1001,
                &request(),
                || 201,
            )
            .await;
    });
    panic!("crash fixture must exit during the effect");
}
#[tokio::test]
async fn process_death_after_effect_before_result_persistence_never_replays_the_effect() {
    let dir = private_dir();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tests::crash_helper", "--nocapture"])
        .env("LIMEOS_RECEIPT_CRASH_FIXTURE", dir.path())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(86));
    assert_eq!(
        std::fs::read(dir.path().join("effect-count")).unwrap(),
        b"1"
    );
    let mut store = ReceiptStore::open(dir.path()).unwrap();
    let engine = FakeEngine::new(Mode::Accept);
    let receipt = store
        .execute(&engine, &ceiling(), 1001, &request(), || 202)
        .await
        .unwrap();
    assert_eq!(receipt.state, ExecutionState::Prepared);
    assert_eq!(engine.effects.load(Ordering::SeqCst), 0);
    assert_eq!(
        std::fs::read(dir.path().join("effect-count")).unwrap(),
        b"1"
    );
}

#[test]
fn symlinked_receipt_lock_is_refused_on_every_architecture() {
    let directory = private_dir();
    let elsewhere = directory.path().join("elsewhere.lock");
    std::os::unix::fs::symlink(&elsewhere, directory.path().join("receipts.lock")).unwrap();
    assert!(ReceiptStore::open(directory.path()).is_err());
    assert!(!elsewhere.exists());
}
#[test]
fn receipt_lock_hold_helper() {
    if std::env::var_os("LIMEOS_RECEIPT_LOCK_HOLD").is_none() {
        return;
    }
    // Stand-in for a child that still shares the store's lock description.
    std::thread::sleep(std::time::Duration::from_secs(60));
}
#[test]
fn a_child_sharing_the_receipt_lock_description_does_not_block_a_reopen_after_drop() {
    use std::os::unix::fs::MetadataExt;
    struct ChildGuard(std::process::Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let identity = |path: &Path| {
        let metadata = std::fs::metadata(path).unwrap();
        (metadata.dev(), metadata.ino())
    };
    let dir = private_dir();
    let lock = dir.path().join("receipts.lock");
    let store = ReceiptStore::open(dir.path()).unwrap();
    // The child keeps the shared description as stdin for its whole life, like a
    // child that has not exec'd yet, so no timing is involved.
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "tests::receipt_lock_hold_helper"])
        .env("LIMEOS_RECEIPT_LOCK_HOLD", "1")
        .stdin(store._lock.0.try_clone().unwrap())
        .stdout(std::process::Stdio::null());
    let child = ChildGuard(command.spawn().unwrap());
    // Close the parent's duplicate: the store's descriptor is the parent's only one.
    drop(command);
    drop(store);
    let stdin = PathBuf::from(format!("/proc/{}/fd/0", child.0.id()));
    assert_eq!(identity(&stdin), identity(&lock));
    assert!(ReceiptStore::open(dir.path()).is_ok());
    assert_eq!(identity(&stdin), identity(&lock));
}
#[test]
fn a_ceiling_naming_root_as_core_authorizes_nothing() {
    let resource = format!("container:{}", "a".repeat(64));
    assert!(
        ceiling()
            .authorize(1001, &resource, ContainerAction::Restart)
            .is_ok()
    );
    let root = RestartCeiling {
        core_uid: 0,
        ..ceiling()
    };
    assert_eq!(
        root.authorize(0, &resource, ContainerAction::Restart),
        Err(Error(ErrorCode::Forbidden))
    );
}
