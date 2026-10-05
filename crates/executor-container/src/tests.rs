use super::*;
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
        managed_containers: vec!["a".repeat(64)],
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
    store.conn.execute_batch("PRAGMA user_version=99;").unwrap();
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
