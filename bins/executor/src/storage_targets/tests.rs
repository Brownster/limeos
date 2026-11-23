//! The target-preparation connection handler over a socket pair, with the
//! ceiling walked beneath a test-owned directory: these fail if
//! authorization moves back to start-up or to the read made at accept.
//!
//! Each test skips itself when run as root, since a root test user cannot
//! be core (`core_uid` 0 is refused); CI runs them unprivileged.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn me() -> u32 {
    rustix::process::geteuid().as_raw()
}

/// A test-owned tree standing in for `/`; removed on drop.
struct Root(PathBuf);
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
impl Root {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = Self(std::env::temp_dir().join(format!(
            "executor-targets-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )));
        for directory in ["", "etc", "etc/limeos", "etc/limeos/system-policy"] {
            let path = root.0.join(directory);
            std::fs::create_dir_all(&path).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        root
    }
    fn write(&self, body: &str) {
        let path = self.0.join(&POLICY[1..]);
        let temporary = path.with_extension("tmp");
        std::fs::write(&temporary, body).unwrap();
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::rename(&temporary, &path).unwrap();
    }
    fn ceilings(&self) -> TargetCeilings {
        TargetCeilings::at(&self.0, Path::new(POLICY), me())
    }
}

fn contract() -> limeos_domain::StorageContract {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/storage-contracts.json"
    ))
    .unwrap();
    serde_json::from_value(fixture["cases"][0]["contract"].clone()).unwrap()
}
fn ceiling_json(core_uid: u32, allow: bool) -> String {
    let targets: Vec<_> = contract()
        .devices
        .iter()
        .map(|d| serde_json::json!({"filesystem_uuid": d.filesystem_uuid, "mountpoint": d.mountpoint}))
        .collect();
    serde_json::json!({
        "version": VERSION,
        "core_uid": core_uid,
        "allow_prepare_targets": allow,
        "managed_targets": targets,
    })
    .to_string()
}
fn inspect() -> Request {
    Request::InspectStorageTargets {
        version: VERSION,
        contract: contract(),
    }
}

async fn connect(
    ceilings: Arc<TargetCeilings>,
    operation: Arc<Semaphore>,
) -> (UnixStream, tokio::task::JoinHandle<()>) {
    let (client, server) = UnixStream::pair().unwrap();
    let task = tokio::spawn(async move { serve(server, &ceilings, &operation).await });
    // Single-threaded runtime: the handler runs to its frame read, after its
    // accept-time ceiling read.
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
    (client, task)
}
async fn send(
    mut client: UnixStream,
    task: tokio::task::JoinHandle<()>,
    request: &Request,
) -> Receipt {
    // A peer refused at accept is answered and closed before its frame, so
    // the write may fail; the refusal is still there to read.
    let _ = write_frame(&mut client, request).await;
    let receipt: Receipt = read_frame(&mut client).await.unwrap();
    task.await.unwrap();
    receipt
}
async fn exchange(ceilings: &Arc<TargetCeilings>, request: &Request) -> Receipt {
    let (client, task) = connect(ceilings.clone(), Arc::new(Semaphore::new(1))).await;
    send(client, task, request).await
}

// An authorized inspection gets past the ceiling and stops at the root
// check inside `target_snapshot` (`Unavailable` for this unprivileged
// test); a refused one stops at the ceiling with `Forbidden`.

#[tokio::test(flavor = "current_thread")]
async fn a_revocation_between_two_requests_refuses_the_second() {
    if me() == 0 {
        return eprintln!("skipped: a root test user cannot be core");
    }
    let root = Root::new();
    root.write(&ceiling_json(me(), true));
    let ceilings = Arc::new(root.ceilings());
    // The ceiling reads cleanly, so this `Unavailable` is the root check
    // past authorization, not a refused ceiling.
    assert!(ceilings.load().is_ok());
    assert_eq!(
        exchange(&ceilings, &inspect()).await.error,
        Some(ErrorCode::Unavailable)
    );
    root.write(&ceiling_json(me(), false));
    assert_eq!(
        exchange(&ceilings, &inspect()).await.error,
        Some(ErrorCode::Forbidden)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_revocation_after_accept_and_before_the_frame_refuses_the_request() {
    if me() == 0 {
        return eprintln!("skipped: a root test user cannot be core");
    }
    let root = Root::new();
    root.write(&ceiling_json(me(), true));
    let ceilings = Arc::new(root.ceilings());
    let (client, task) = connect(ceilings.clone(), Arc::new(Semaphore::new(1))).await;
    assert!(!task.is_finished(), "the handler waits for the frame");
    root.write(&ceiling_json(me(), false));
    assert_eq!(
        send(client, task, &inspect()).await.error,
        Some(ErrorCode::Forbidden)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_root_core_a_broken_file_or_another_peer_is_refused() {
    if me() == 0 {
        return eprintln!("skipped: a root test user cannot be core");
    }
    let root = Root::new();
    let ceilings = Arc::new(root.ceilings());
    root.write(&ceiling_json(0, true));
    assert_eq!(ceilings.load().err(), Some(Refusal::RootCore));
    assert_eq!(
        exchange(&ceilings, &inspect()).await.error,
        Some(ErrorCode::Unavailable)
    );
    root.write("{");
    assert_eq!(
        ceilings.load().err(),
        Some(Refusal::Read(
            limeos_executor_storage::ProtectedRead::Malformed
        ))
    );
    assert_eq!(
        exchange(&ceilings, &inspect()).await.error,
        Some(ErrorCode::Unavailable)
    );
    root.write(&ceiling_json(me() + 1, true));
    assert_eq!(
        exchange(&ceilings, &inspect()).await.error,
        Some(ErrorCode::Forbidden)
    );
}
